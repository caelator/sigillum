//! CLI bridge to the Sigillum daemon's HTTP API.
//!
//! Translates `sigillum api <COMMAND>` invocations into REST calls via
//! [`SigillumClient`], providing a scriptable, non-interactive interface
//! suitable for shell pipelines, CI, and automated key ceremonies.
//!
//! ## Design
//!
//! Every subcommand follows the same pattern: parse flags from `args`,
//! construct the typed request, call [`run_api_command`] which builds a
//! one-shot Tokio runtime and prints pretty-printed JSON to stdout.
//! Errors go to stderr and exit non-zero.
//!
//! Session tokens are resolved in priority order:
//! 1. `--session <TOKEN>` flag
//! 2. `SIGILLUM_SESSION_TOKEN` environment variable
//!
//! Sensitive passphrases and mnemonics support three delivery modes:
//! `--*-env VAR` (read from environment), `--*-stdin` (read from stdin),
//! or interactive terminal prompt via `rpassword`. Optional FIDO2 PINs are
//! accepted through `--pin-env` or `--pin-stdin` when a specific key requires
//! one; otherwise the touch-only path sends no PIN at all.
//!
//! `profiles eth-seed create` never prints the freshly generated mnemonic by
//! default: the JSON output carries a redacted placeholder unless the operator
//! reveals it on an interactive terminal (`--reveal-mnemonic`) or files it
//! away with owner-only permissions (`--mnemonic-out <PATH>`).

use std::future::Future;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::{self, Stdio};
use std::time::{Duration, Instant};

use serde::Serialize;
use sigillum_api::request::{
    EthSeedWalletCreateRequest, EthSeedWalletProfileUpsertRequest,
    EthStealthWalletProfileUpsertRequest, EthXpubWalletProfileUpsertRequest,
    EvmProviderProfileUpsertRequest, EvmProviderRef, Fido2UnlockRequest, MaintenanceRunRequest,
    RiskCatalogDeleteRequest, RiskCatalogUpsertRequest, SelfCheckRunRequest,
};
use sigillum_api::response::EthSeedWalletCreateResponse;
use sigillum_client::{ClientError, SigillumClient};
use url::Url;

mod args;
mod deposits;
mod evm;
mod inventory;
mod inventory_args;
mod plans;
mod queue;
mod receiving;
mod transit;
mod treasury;
mod wallets;

pub(crate) use args::*;

const DEFAULT_DAEMON_BASE_URL: &str = "http://127.0.0.1:9743";
const DAEMON_READY_TIMEOUT: Duration = Duration::from_secs(10);
const DAEMON_READY_POLL_INTERVAL: Duration = Duration::from_millis(200);

/// Dispatch a `sigillum api <COMMAND>` invocation.
pub fn cmd_api(args: &[String]) {
    if args.is_empty() {
        print_api_usage();
        process::exit(1);
    }

    match args[0].as_str() {
        "status" => run_api_command(args, false, |client| async move { client.status().await }),
        "unlock" => {
            let passphrase = read_passphrase(args);
            run_api_command(args, false, move |client| async move {
                client.unlock_with_passphrase(&passphrase).await
            });
        }
        "unlock-fido2" => {
            let tap_count = require_usize_flag(
                args,
                "--taps",
                "sigillum api unlock-fido2 --taps <N> [--pin-env VAR|--pin-stdin]",
            );
            let pins: Vec<String> = read_optional_pin(args).into_iter().collect();
            run_api_command(args, false, move |client| async move {
                client
                    .fido2_unlock(Fido2UnlockRequest { pins, tap_count })
                    .await
            });
        }
        "lock" => run_api_command(args, true, |client| async move { client.lock().await }),
        "revoke-session" => {
            run_api_command(
                args,
                true,
                |client| async move { client.revoke_session().await },
            )
        }
        "switch" => {
            let id = require_usize_flag(args, "--id", "sigillum api switch --id <N>");
            run_api_command(args, true, move |client| async move {
                client.switch_compartment(id).await
            });
        }
        "compartment" => cmd_api_compartment(args),
        "diagnostics" => run_api_command(
            args,
            true,
            |client| async move { client.diagnostics().await },
        ),
        "selfcheck" => {
            let request = SelfCheckRunRequest {
                domains: parse_multi_flag(args, "--domain"),
            };
            run_api_command(args, true, move |client| async move {
                client.run_self_check(request).await
            });
        }
        "profiles" => cmd_api_profiles(args),
        "deposits" => deposits::cmd_api_deposits(args),
        "evm" => evm::cmd_api_evm(args),
        "chains" => inventory::cmd_api_chains(args),
        "inventory" => inventory::cmd_api_inventory(args),
        "discovery" => cmd_api_discovery(args),
        "risk" => cmd_api_risk(args),
        "plans" => plans::cmd_api_plans(args),
        "receiving" => receiving::cmd_api_receiving(args),
        "treasury" => treasury::cmd_api_treasury(args),
        "queue" => queue::cmd_api_queue(args),
        "transit" => transit::cmd_api_transit(args),
        "wallets" => wallets::cmd_api_wallets(args),
        "maintenance" => cmd_api_maintenance(args),
        "help" | "--help" | "-h" => print_api_usage(),
        other => {
            eprintln!("Unknown api command: {other}");
            print_api_usage();
            process::exit(1);
        }
    }
}

/// Dispatch `sigillum api compartment <list>`.
fn cmd_api_compartment(args: &[String]) {
    match args.get(1).map(String::as_str) {
        Some("list") => run_api_command(args, true, |client| async move {
            client.list_compartments().await
        }),
        _ => {
            eprintln!("Usage: sigillum api compartment <list>");
            process::exit(1);
        }
    }
}

/// Dispatch `sigillum api profiles <evm|stealth|eth-xpub|eth-seed> <list|upsert|create|delete>`.
fn cmd_api_profiles(args: &[String]) {
    if args.len() < 3 {
        eprintln!(
            "Usage: sigillum api profiles <evm|stealth|eth-xpub|eth-seed> <list|upsert|create|delete> [...]"
        );
        process::exit(1);
    }

    match (args[1].as_str(), args[2].as_str()) {
        ("evm", "list") => run_api_command(args, true, |client| async move {
            client.list_evm_provider_profiles().await
        }),
        ("evm", "upsert") => {
            let request = EvmProviderProfileUpsertRequest {
                name: require_flag(
                    args,
                    "--name",
                    "sigillum api profiles evm upsert --name <NAME> --rpc-url <URL> --chain-id <N> [--fee-estimation|--no-fee-estimation]",
                ),
                provider: EvmProviderRef {
                    rpc_url: require_flag(
                        args,
                        "--rpc-url",
                        "sigillum api profiles evm upsert --name <NAME> --rpc-url <URL> --chain-id <N> [--fee-estimation|--no-fee-estimation]",
                    ),
                    auth_token_key: parse_flag(args, "--auth-token-key"),
                    compartment_id: parse_usize_flag(args, "--compartment-id"),
                },
                chain_id: require_u64_flag(
                    args,
                    "--chain-id",
                    "sigillum api profiles evm upsert --name <NAME> --rpc-url <URL> --chain-id <N> [--fee-estimation|--no-fee-estimation]",
                ),
                max_priority_fee_per_gas_hex: parse_flag(args, "--max-priority-fee-per-gas-hex"),
                max_fee_per_gas_hex: parse_flag(args, "--max-fee-per-gas-hex"),
                native_gas_limit: parse_u64_flag(args, "--native-gas-limit"),
                erc20_gas_limit: parse_u64_flag(args, "--erc20-gas-limit"),
                fee_estimation_enabled: bool_switch(
                    args,
                    "--fee-estimation",
                    "--no-fee-estimation",
                ),
            };
            run_api_command(args, true, move |client| async move {
                client.upsert_evm_provider_profile(request).await
            });
        }
        ("evm", "delete") => {
            let name = require_flag(
                args,
                "--name",
                "sigillum api profiles evm delete --name <NAME>",
            );
            run_api_command(args, true, move |client| async move {
                client.delete_evm_provider_profile(&name).await
            });
        }
        ("stealth", "list") => run_api_command(args, true, |client| async move {
            client.list_eth_stealth_wallet_profiles().await
        }),
        ("stealth", "upsert") => {
            let request = EthStealthWalletProfileUpsertRequest {
                name: require_flag(
                    args,
                    "--name",
                    "sigillum api profiles stealth upsert --name <NAME> --wallet <WALLET> --provider-profile <PROFILE>",
                ),
                wallet: require_flag(
                    args,
                    "--wallet",
                    "sigillum api profiles stealth upsert --name <NAME> --wallet <WALLET> --provider-profile <PROFILE>",
                ),
                short_name: parse_flag(args, "--short-name"),
                provider_profile: require_flag(
                    args,
                    "--provider-profile",
                    "sigillum api profiles stealth upsert --name <NAME> --wallet <WALLET> --provider-profile <PROFILE>",
                ),
                compartment_id: parse_usize_flag(args, "--compartment-id"),
                chain_id: parse_u64_flag(args, "--chain-id"),
                default_destination_address: parse_flag(args, "--default-destination-address"),
                execution_enabled: bool_switch(args, "--execution-enabled", "--execution-disabled"),
            };
            run_api_command(args, true, move |client| async move {
                client.upsert_eth_stealth_wallet_profile(request).await
            });
        }
        ("stealth", "delete") => {
            let name = require_flag(
                args,
                "--name",
                "sigillum api profiles stealth delete --name <NAME>",
            );
            run_api_command(args, true, move |client| async move {
                client.delete_eth_stealth_wallet_profile(&name).await
            });
        }
        ("eth-xpub", "list") => run_api_command(args, true, |client| async move {
            client.list_eth_xpub_wallet_profiles().await
        }),
        ("eth-xpub", "upsert") => {
            const XPUB_USAGE: &str = "sigillum api profiles eth-xpub upsert --name <NAME> --provider-profile <PROFILE> [--project-account <N>] [--compartment-id <N>] [--chain-id <N>] [--external-receive-xpub <XPUB>] [--external-receive-path <PATH>] [--external-account-xpub <XPUB>] [--external-account-path <PATH>] [--default-destination-address <ADDR>] [--execution-enabled|--execution-disabled]";
            let request = EthXpubWalletProfileUpsertRequest {
                name: require_flag(args, "--name", XPUB_USAGE),
                project_account: parse_u32_flag(args, "--project-account").unwrap_or(0),
                provider_profile: require_flag(args, "--provider-profile", XPUB_USAGE),
                compartment_id: parse_usize_flag(args, "--compartment-id"),
                chain_id: parse_u64_flag(args, "--chain-id"),
                external_receive_xpub: parse_flag(args, "--external-receive-xpub"),
                external_receive_path: parse_flag(args, "--external-receive-path"),
                external_account_xpub: parse_flag(args, "--external-account-xpub"),
                external_account_path: parse_flag(args, "--external-account-path"),
                default_destination_address: parse_flag(args, "--default-destination-address"),
                execution_enabled: bool_switch(args, "--execution-enabled", "--execution-disabled"),
            };
            run_api_command(args, true, move |client| async move {
                client.upsert_eth_xpub_wallet_profile(request).await
            });
        }
        ("eth-xpub", "delete") => {
            let name = require_flag(
                args,
                "--name",
                "sigillum api profiles eth-xpub delete --name <NAME>",
            );
            run_api_command(args, true, move |client| async move {
                client.delete_eth_xpub_wallet_profile(&name).await
            });
        }
        ("eth-seed", "list") => run_api_command(args, true, |client| async move {
            client.list_eth_seed_wallet_profiles().await
        }),
        ("eth-seed", "create") => {
            const CREATE_USAGE: &str = "sigillum api profiles eth-seed create --name <NAME> --provider-profile <PROFILE> [--word-count 12|24] [--label <LABEL>] [--project-account <N>] [--compartment-id <N>] [--chain-id <N>] [--default-destination-address <ADDR>] [--mnemonic-passphrase-env VAR|--mnemonic-passphrase-stdin] [--reveal-mnemonic|--mnemonic-out <PATH>]";
            let request = EthSeedWalletCreateRequest {
                name: require_flag(args, "--name", CREATE_USAGE),
                label: parse_flag(args, "--label"),
                word_count: parse_usize_flag(args, "--word-count"),
                mnemonic_passphrase: read_optional_mnemonic_passphrase(args),
                project_account: parse_u32_flag(args, "--project-account").unwrap_or(0),
                provider_profile: require_flag(args, "--provider-profile", CREATE_USAGE),
                compartment_id: parse_usize_flag(args, "--compartment-id"),
                chain_id: parse_u64_flag(args, "--chain-id"),
                default_destination_address: parse_flag(args, "--default-destination-address"),
                execution_enabled: bool_switch(args, "--execution-enabled", "--execution-disabled"),
            };
            run_eth_seed_create(args, request);
        }
        ("eth-seed", "upsert") => {
            const UPSERT_USAGE: &str = "sigillum api profiles eth-seed upsert --name <NAME> --provider-profile <PROFILE> [--mnemonic-env VAR|--mnemonic-stdin] [--label <LABEL>] [--project-account <N>] [--compartment-id <N>] [--chain-id <N>] [--default-destination-address <ADDR>] [--mnemonic-passphrase-env VAR|--mnemonic-passphrase-stdin] [--execution-enabled|--execution-disabled]";
            let request = EthSeedWalletProfileUpsertRequest {
                name: require_flag(args, "--name", UPSERT_USAGE),
                provider_profile: require_flag(args, "--provider-profile", UPSERT_USAGE),
                label: parse_flag(args, "--label"),
                mnemonic: read_mnemonic(args),
                mnemonic_passphrase: read_optional_mnemonic_passphrase(args),
                project_account: parse_u32_flag(args, "--project-account").unwrap_or(0),
                compartment_id: parse_usize_flag(args, "--compartment-id"),
                chain_id: parse_u64_flag(args, "--chain-id"),
                default_destination_address: parse_flag(args, "--default-destination-address"),
                execution_enabled: bool_switch(args, "--execution-enabled", "--execution-disabled"),
            };
            run_api_command(args, true, move |client| async move {
                client.upsert_eth_seed_wallet_profile(request).await
            });
        }
        ("eth-seed", "delete") => {
            let name = require_flag(
                args,
                "--name",
                "sigillum api profiles eth-seed delete --name <NAME>",
            );
            run_api_command(args, true, move |client| async move {
                client.delete_eth_seed_wallet_profile(&name).await
            });
        }
        _ => {
            eprintln!(
                "Usage: sigillum api profiles <evm|stealth|eth-xpub|eth-seed> <list|upsert|create|delete> [...]"
            );
            process::exit(1);
        }
    }
}

/// Dispatch `sigillum api discovery <jobs|scan-evm>`.
fn cmd_api_discovery(args: &[String]) {
    if args.len() < 2 {
        eprintln!("Usage: sigillum api discovery <jobs|scan-evm> [...]");
        process::exit(1);
    }

    match args[1].as_str() {
        "jobs" => {
            if args.len() < 3 {
                eprintln!("Usage: sigillum api discovery jobs <list|cancel|resume> [...]");
                process::exit(1);
            }
            match args[2].as_str() {
                "list" => run_api_command(args, true, |client| async move {
                    client.list_discovery_jobs().await
                }),
                "cancel" => {
                    let id = require_flag(
                        args,
                        "--id",
                        "sigillum api discovery jobs cancel --id <JOB_ID>",
                    );
                    run_api_command(args, true, move |client| async move {
                        client.cancel_discovery_job(&id).await
                    });
                }
                "resume" => {
                    let id = require_flag(
                        args,
                        "--id",
                        "sigillum api discovery jobs resume --id <JOB_ID>",
                    );
                    run_api_command(args, true, move |client| async move {
                        client.resume_discovery_job(&id).await
                    });
                }
                _ => {
                    eprintln!("Usage: sigillum api discovery jobs <list|cancel|resume> [...]");
                    process::exit(1);
                }
            }
        }
        "scan-evm" => inventory::cmd_api_inventory(
            &["inventory".into(), args[1].clone()]
                .into_iter()
                .chain(args.iter().skip(2).cloned())
                .collect::<Vec<_>>(),
        ),
        _ => {
            eprintln!("Usage: sigillum api discovery <jobs|scan-evm> [...]");
            process::exit(1);
        }
    }
}

/// Dispatch `sigillum api risk <list|catalog|catalog-upsert|catalog-delete>`.
fn cmd_api_risk(args: &[String]) {
    match args.get(1).map(String::as_str) {
        Some("list") => run_api_command(args, true, |client| async move {
            client.list_risk_findings().await
        }),
        Some("catalog") => run_api_command(args, true, |client| async move {
            client.list_risk_catalog().await
        }),
        Some("catalog-upsert") => {
            let request = RiskCatalogUpsertRequest {
                address: require_flag(
                    args,
                    "--address",
                    "sigillum api risk catalog-upsert --address <ADDRESS> --risk-level <LEVEL>",
                ),
                label: parse_flag(args, "--label"),
                risk_level: require_flag(
                    args,
                    "--risk-level",
                    "sigillum api risk catalog-upsert --address <ADDRESS> --risk-level <LEVEL>",
                ),
                notes: parse_multi_flag(args, "--note"),
            };
            run_api_command(args, true, move |client| async move {
                client.upsert_risk_catalog_entry(request).await
            });
        }
        Some("catalog-delete") => {
            let address = require_flag(
                args,
                "--address",
                "sigillum api risk catalog-delete --address <ADDRESS>",
            );
            run_api_command(args, true, move |client| async move {
                client
                    .delete_risk_catalog_entry(RiskCatalogDeleteRequest { address })
                    .await
            });
        }
        _ => {
            eprintln!(
                "Usage: sigillum api risk <list|catalog|catalog-upsert|catalog-delete> [...]"
            );
            process::exit(1);
        }
    }
}

/// Dispatch `sigillum api maintenance run [...]`.
fn cmd_api_maintenance(args: &[String]) {
    if args.len() < 2 || args[1].as_str() != "run" {
        eprintln!(
            "Usage: sigillum api maintenance run [--deposit-refresh-limit N] [--queue-process-limit N] [--auto-enqueue|--no-auto-enqueue]"
        );
        process::exit(1);
    }

    let request = MaintenanceRunRequest {
        deposit_refresh_limit: parse_usize_flag(args, "--deposit-refresh-limit"),
        queue_process_limit: parse_usize_flag(args, "--queue-process-limit"),
        auto_enqueue: bool_switch(args, "--auto-enqueue", "--no-auto-enqueue"),
    };
    run_api_command(args, true, move |client| async move {
        client.run_maintenance(request).await
    });
}

// ── Runtime and client construction ─────────────────────────────

/// Execute an API call within a one-shot Tokio runtime.
///
/// Builds a [`SigillumClient`], optionally attaches a session token,
/// runs the async closure, and prints the JSON result to stdout.
/// Exits non-zero on any error.
fn run_api_command<T, F, Fut>(args: &[String], require_session: bool, f: F)
where
    T: Serialize,
    F: FnOnce(SigillumClient) -> Fut,
    Fut: Future<Output = Result<T, ClientError>>,
{
    run_api_command_with(args, require_session, f, |_| {});
}

/// Like [`run_api_command`], but invokes `inspect` on the response before it
/// is printed, so callers can surface non-blocking findings (e.g. response
/// warnings) on stderr without polluting the JSON stdout.
fn run_api_command_with<T, F, Fut, I>(args: &[String], require_session: bool, f: F, inspect: I)
where
    T: Serialize,
    F: FnOnce(SigillumClient) -> Fut,
    Fut: Future<Output = Result<T, ClientError>>,
    I: FnOnce(&T),
{
    let client = build_client(args, require_session);
    let runtime = tokio::runtime::Runtime::new().unwrap_or_else(|error| {
        eprintln!("Failed to start async runtime: {error}");
        process::exit(1);
    });
    let response = match runtime.block_on(f(client)) {
        Ok(response) => response,
        Err(error) => report_client_error(error),
    };
    inspect(&response);
    print_json(&response);
}

/// Construct a [`SigillumClient`] with optional session-token attachment.
pub(super) fn build_client(args: &[String], require_session: bool) -> SigillumClient {
    let client = SigillumClient::new(daemon_base_url(args)).unwrap_or_else(|error| {
        eprintln!("Failed to build daemon client: {error}");
        process::exit(1);
    });
    if require_session {
        let session = require_session_token(args);
        client.set_session_token(session);
    }
    client
}

/// Resolve the daemon base URL from `--url` flag or `SIGILLUM_BASE_URL` env.
pub(crate) fn daemon_base_url(args: &[String]) -> String {
    parse_flag(args, "--url")
        .or_else(|| std::env::var("SIGILLUM_BASE_URL").ok())
        .or_else(|| std::env::var("SIGILLUM_DAEMON_URL").ok())
        .unwrap_or_else(|| DEFAULT_DAEMON_BASE_URL.into())
}

pub(crate) fn session_token_from_args(args: &[String]) -> Option<String> {
    parse_flag(args, "--session").or_else(|| std::env::var("SIGILLUM_SESSION_TOKEN").ok())
}

pub(crate) fn require_session_token(args: &[String]) -> String {
    session_token_from_args(args).unwrap_or_else(|| {
        eprintln!(
            "This command requires a daemon session token. Use --session <TOKEN> or set SIGILLUM_SESSION_TOKEN."
        );
        process::exit(1);
    })
}

pub(crate) fn ensure_daemon_ready(base_url: &str) -> Result<(), String> {
    let runtime = tokio::runtime::Runtime::new().map_err(|error| error.to_string())?;
    runtime.block_on(async move {
        if daemon_reachable(base_url).await {
            return Ok(());
        }

        let daemon_url = Url::parse(base_url).map_err(|error| error.to_string())?;
        let host = daemon_url.host_str().unwrap_or_default();
        if host != "127.0.0.1" && host != "localhost" {
            return Err(format!(
                "daemon at {base_url} is unreachable and cannot be auto-started for non-local hosts"
            ));
        }

        let port = daemon_url
            .port_or_known_default()
            .ok_or_else(|| format!("daemon url {base_url} is missing a port"))?;
        spawn_daemon_process(port).map_err(|error| error.to_string())?;

        let deadline = Instant::now() + DAEMON_READY_TIMEOUT;
        while Instant::now() < deadline {
            if daemon_reachable(base_url).await {
                return Ok(());
            }
            tokio::time::sleep(DAEMON_READY_POLL_INTERVAL).await;
        }

        Err(format!(
            "daemon at {base_url} did not become ready within {}s",
            DAEMON_READY_TIMEOUT.as_secs()
        ))
    })
}

async fn daemon_reachable(base_url: &str) -> bool {
    match SigillumClient::new(base_url.to_string()) {
        Ok(client) => client.status().await.is_ok(),
        Err(_) => false,
    }
}

fn spawn_daemon_process(port: u16) -> io::Result<()> {
    let current_exe = std::env::current_exe()?;
    let mut command = std::process::Command::new(current_exe);
    command
        .arg("daemon")
        .arg("--port")
        .arg(port.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let _child = command.spawn()?;
    Ok(())
}

// ── Output and error handling ────────────────────────────────────

pub(super) fn print_json<T: Serialize>(value: &T) {
    let body = serde_json::to_string_pretty(value).unwrap_or_else(|error| {
        eprintln!("Failed to encode JSON output: {error}");
        process::exit(1);
    });
    println!("{body}");
}

// ── Seed-phrase output hygiene ───────────────────────────────────

/// Placeholder substituted for the BIP-39 phrase in `eth-seed create` JSON
/// output unless the operator explicitly reveals or files it away.
const MNEMONIC_REDACTED_PLACEHOLDER: &str =
    "<redacted: use --reveal-mnemonic or --mnemonic-out PATH>";

/// How the freshly generated mnemonic of `profiles eth-seed create` may leave
/// the CLI. The phrase never reaches stdout scrollback by default: pipelines
/// and interactive shells alike get the redacted placeholder unless the
/// operator opts into an explicit disclosure channel.
#[derive(Debug, Clone, PartialEq, Eq)]
struct MnemonicOutputPlan {
    /// Print the phrase in the stdout JSON (interactive terminal only).
    reveal_on_stdout: bool,
    /// Write the phrase to this path with 0600 permissions (must not exist).
    out_path: Option<PathBuf>,
}

/// Resolve `--reveal-mnemonic` / `--mnemonic-out <PATH>` into a
/// [`MnemonicOutputPlan`]. Revealing on stdout requires an interactive
/// terminal; scripts and pipelines must use `--mnemonic-out` so the phrase
/// never travels through captured stdout.
fn plan_mnemonic_output(args: &[String], stdout_tty: bool) -> Result<MnemonicOutputPlan, String> {
    let reveal_on_stdout = has_flag(args, "--reveal-mnemonic");
    let out_path = parse_flag(args, "--mnemonic-out").map(PathBuf::from);
    if reveal_on_stdout && !stdout_tty {
        return Err(
            "--reveal-mnemonic requires an interactive terminal; in scripts and pipelines use --mnemonic-out <PATH> instead"
                .to_string(),
        );
    }
    Ok(MnemonicOutputPlan {
        reveal_on_stdout,
        out_path,
    })
}

fn stdout_is_tty() -> bool {
    io::stdout().is_terminal()
}

/// Split the phrase out of a create response according to the disclosure
/// plan: redacted in place unless revealed on stdout. Returns the phrase
/// itself so the caller can file it away before printing the response.
fn split_mnemonic_for_output(
    response: &mut EthSeedWalletCreateResponse,
    plan: &MnemonicOutputPlan,
) -> String {
    let mnemonic = std::mem::take(&mut response.mnemonic);
    response.mnemonic = if plan.reveal_on_stdout {
        mnemonic.clone()
    } else {
        MNEMONIC_REDACTED_PLACEHOLDER.to_string()
    };
    mnemonic
}

/// Write `mnemonic` to `path` with owner-only (0600) permissions, refusing to
/// overwrite an existing file.
fn write_mnemonic_file(path: &Path, mnemonic: &str) -> io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(mnemonic.as_bytes())?;
    file.write_all(b"\n")
}

/// Run `profiles eth-seed create` with mnemonic hygiene: the freshly
/// generated phrase is redacted from the stdout JSON unless the operator
/// reveals it on a terminal (`--reveal-mnemonic`) or files it away
/// (`--mnemonic-out <PATH>`).
fn run_eth_seed_create(args: &[String], request: EthSeedWalletCreateRequest) {
    let plan = plan_mnemonic_output(args, stdout_is_tty()).unwrap_or_else(|error| {
        eprintln!("{error}");
        process::exit(1);
    });
    // Fail fast before the daemon mints a phrase we cannot deliver.
    if let Some(path) = plan.out_path.as_deref()
        && path.exists()
    {
        eprintln!(
            "Refusing to overwrite existing file {}. Choose a new --mnemonic-out path.",
            path.display()
        );
        process::exit(1);
    }

    let client = build_client(args, true);
    let runtime = tokio::runtime::Runtime::new().unwrap_or_else(|error| {
        eprintln!("Failed to start async runtime: {error}");
        process::exit(1);
    });
    let mut response = match runtime.block_on(client.create_eth_seed_wallet_profile(request)) {
        Ok(response) => response,
        Err(error) => report_client_error(error),
    };

    let mnemonic = split_mnemonic_for_output(&mut response, &plan);
    if let Some(path) = plan.out_path.as_deref() {
        if let Err(error) = write_mnemonic_file(path, &mnemonic) {
            eprintln!(
                "Profile '{}' was created, but writing the mnemonic to {} failed: {error}. The phrase was not printed; it remains stored only as an encrypted daemon vault secret.",
                response.profile.name,
                path.display()
            );
            process::exit(1);
        }
        eprintln!("Mnemonic written to {} (mode 0600).", path.display());
    }
    if !plan.reveal_on_stdout {
        eprintln!(
            "note: the new mnemonic is redacted from stdout; use --reveal-mnemonic on a terminal or --mnemonic-out <PATH> to back it up."
        );
    }
    print_json(&response);
}

pub(super) fn report_client_error(error: ClientError) -> ! {
    eprintln!("{error}");
    process::exit(1);
}

fn print_api_usage() {
    eprintln!(
        "\
sigillum api <COMMAND>

COMMANDS:
  status
  unlock [--passphrase-env VAR|--passphrase-stdin]
  unlock-fido2 --taps <N> [--pin-env VAR|--pin-stdin]
  lock
  revoke-session
  switch --id <N>
  compartment <list>
  diagnostics
  selfcheck [--domain <DOMAIN>]...  (domains: provider, seed-wallet, xpub-wallet, stealth-wallet, watch-book, policy, receive-allocation, fido2; default: all)
  profiles evm <list|upsert|delete> [...]
  profiles stealth <list|upsert|delete> [...]
  profiles eth-xpub <list|upsert|delete> [...]
  profiles eth-seed <list|upsert|create|delete> [...]  (upsert imports an existing mnemonic via --mnemonic-env VAR|--mnemonic-stdin or hidden prompt; create generates a new BIP-39 mnemonic, redacted from stdout unless --reveal-mnemonic or filed via --mnemonic-out <PATH>)
  deposits <list|create-native|create-erc20|scan-announcements|refresh|enqueue-sweep|delete> [...]
  evm <nonce|balance|erc20-balance|fees> [...]  (read-only; no broadcast)
  chains <list|upsert|delete> [...]
  inventory <list|chains|watch|token-registry|scan-evm> [...]  (scan supports --watch-address, --watch-address-file, --include-watch-book, --derivation-pattern, --account-limit, --probe-token-registry)
  discovery <jobs|scan-evm> [...]
  risk <list|catalog|catalog-upsert|catalog-delete> [...]
  plans <list|generate|approve|simulate|export|enqueue-step|enqueue-plan> [...]  (enqueue-step needs --confirm; enqueue-plan needs --confirmation <PHRASE>)
  receiving <overview|refresh-balances|tag-deposit> [...]
  treasury <overview|policy|policy-update|receive-list|receive-allocate|receive-rotate|parties> [...]
  queue <list|process|pause|resume> [...]
  transit <encrypt|decrypt|hmac> [...]
  wallets <xpub-export|xpub-derive|stealth-export|stealth-generate|stealth-check> [...]  (read/derive only; no sign/send)
  maintenance run [...]

GLOBAL FLAGS:
  --url <BASE_URL>        Override daemon URL (default: SIGILLUM_BASE_URL or http://127.0.0.1:9743)
  --session <TOKEN>       Override daemon session token (default: SIGILLUM_SESSION_TOKEN)"
    );
}

// ── Tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use sigillum_api::response::EthSeedWalletProfile;

    const TEST_MNEMONIC: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    fn create_response() -> EthSeedWalletCreateResponse {
        EthSeedWalletCreateResponse {
            status: "created".into(),
            mnemonic: TEST_MNEMONIC.into(),
            profile: EthSeedWalletProfile {
                name: "ops-seed".into(),
                label: None,
                project_account: 0,
                provider_profile: "mainnet".into(),
                compartment_id: 0,
                chain_id: Some(1),
                word_count: 12,
                mnemonic_secret_key: "seed-wallet/ops-seed".into(),
                account_path: "m/44'/60'/0'".into(),
                receive_path: "m/44'/60'/0'/0".into(),
                receive_xpub: "xpub...".into(),
                first_receive_address: "0x9858EfFD232B4033E47d90003D41EC34EcaEda94".into(),
                default_destination_address: None,
                control_xpub: None,
                sponsor_address: None,
                hot_address: None,
                treasury_address: None,
                execution_enabled: false,
            },
        }
    }

    // ── MnemonicOutputPlan ───────────────────────────────────────

    #[test]
    fn plan_mnemonic_output_redacts_by_default_on_tty() {
        let plan = plan_mnemonic_output(&args(&[]), true).unwrap();
        assert_eq!(
            plan,
            MnemonicOutputPlan {
                reveal_on_stdout: false,
                out_path: None,
            }
        );
    }

    #[test]
    fn plan_mnemonic_output_redacts_by_default_off_tty() {
        let plan = plan_mnemonic_output(&args(&[]), false).unwrap();
        assert!(!plan.reveal_on_stdout);
        assert_eq!(plan.out_path, None);
    }

    #[test]
    fn plan_mnemonic_output_reveal_allowed_on_tty() {
        let plan = plan_mnemonic_output(&args(&["--reveal-mnemonic"]), true).unwrap();
        assert!(plan.reveal_on_stdout);
        assert_eq!(plan.out_path, None);
    }

    #[test]
    fn plan_mnemonic_output_reveal_rejected_off_tty() {
        let error = plan_mnemonic_output(&args(&["--reveal-mnemonic"]), false).unwrap_err();
        assert!(
            error.contains("--mnemonic-out"),
            "error should point scripts at --mnemonic-out: {error}"
        );
    }

    #[test]
    fn plan_mnemonic_output_file_only() {
        for stdout_tty in [true, false] {
            let plan =
                plan_mnemonic_output(&args(&["--mnemonic-out", "/tmp/seed.txt"]), stdout_tty)
                    .unwrap();
            assert!(!plan.reveal_on_stdout);
            assert_eq!(plan.out_path, Some(PathBuf::from("/tmp/seed.txt")));
        }
    }

    #[test]
    fn plan_mnemonic_output_reveal_and_file_on_tty() {
        let plan = plan_mnemonic_output(
            &args(&["--reveal-mnemonic", "--mnemonic-out", "/tmp/seed.txt"]),
            true,
        )
        .unwrap();
        assert!(plan.reveal_on_stdout);
        assert_eq!(plan.out_path, Some(PathBuf::from("/tmp/seed.txt")));
    }

    #[test]
    fn plan_mnemonic_output_reveal_and_file_rejected_off_tty() {
        let error = plan_mnemonic_output(
            &args(&["--reveal-mnemonic", "--mnemonic-out", "/tmp/seed.txt"]),
            false,
        )
        .unwrap_err();
        assert!(error.contains("--mnemonic-out"));
    }

    // ── Redaction ────────────────────────────────────────────────

    #[test]
    fn split_mnemonic_for_output_redacts_by_default() {
        let plan = MnemonicOutputPlan {
            reveal_on_stdout: false,
            out_path: None,
        };
        let mut response = create_response();
        let mnemonic = split_mnemonic_for_output(&mut response, &plan);
        assert_eq!(mnemonic, TEST_MNEMONIC);
        assert_eq!(response.mnemonic, MNEMONIC_REDACTED_PLACEHOLDER);
        assert_eq!(response.profile.name, "ops-seed");
        assert!(!response.mnemonic.contains("abandon"));
    }

    #[test]
    fn split_mnemonic_for_output_reveal_keeps_phrase() {
        let plan = MnemonicOutputPlan {
            reveal_on_stdout: true,
            out_path: None,
        };
        let mut response = create_response();
        let mnemonic = split_mnemonic_for_output(&mut response, &plan);
        assert_eq!(mnemonic, TEST_MNEMONIC);
        assert_eq!(response.mnemonic, TEST_MNEMONIC);
    }

    // ── Mnemonic file output ─────────────────────────────────────

    #[test]
    fn write_mnemonic_file_creates_owner_only_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mnemonic.txt");
        write_mnemonic_file(&path, TEST_MNEMONIC).unwrap();
        let body = std::fs::read_to_string(&path).unwrap();
        assert_eq!(body, format!("{TEST_MNEMONIC}\n"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "mnemonic file must be owner-only");
        }
    }

    #[test]
    fn write_mnemonic_file_refuses_to_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mnemonic.txt");
        std::fs::write(&path, "existing").unwrap();
        let error = write_mnemonic_file(&path, TEST_MNEMONIC).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "existing");
    }
}
