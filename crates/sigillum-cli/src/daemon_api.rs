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
//! Sensitive passphrases support three delivery modes:
//! `--*-env VAR` (read from environment), `--*-stdin` (read from stdin),
//! or interactive terminal prompt via `rpassword`. Optional FIDO2 PINs are
//! accepted through `--pin-env` or `--pin-stdin` when a specific key requires
//! one; otherwise the touch-only path sends no PIN at all.

use std::future::Future;
use std::io;
use std::process::{self, Stdio};
use std::time::{Duration, Instant};

use serde::Serialize;
use sigillum_api::request::{
    ConsolidationPlanApproveRequest, ConsolidationPlanExportRequest,
    ConsolidationPlanGenerateRequest, ConsolidationPlanSimulateRequest, EthSeedWalletCreateRequest,
    EthStealthWalletProfileUpsertRequest, EthXpubWalletProfileUpsertRequest,
    EvmProviderProfileUpsertRequest, EvmProviderRef, Fido2UnlockRequest, MaintenanceRunRequest,
    PartyDestination, RiskCatalogDeleteRequest, RiskCatalogUpsertRequest, SelfCheckRunRequest,
};
use sigillum_client::{ClientError, SigillumClient};
use url::Url;

mod args;
mod deposits;
mod evm;
mod inventory;
mod inventory_args;
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
        "plans" => cmd_api_plans(args),
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
                    "sigillum api profiles evm upsert --name <NAME> --rpc-url <URL> --chain-id <N>",
                ),
                provider: EvmProviderRef {
                    rpc_url: require_flag(
                        args,
                        "--rpc-url",
                        "sigillum api profiles evm upsert --name <NAME> --rpc-url <URL> --chain-id <N>",
                    ),
                    auth_token_key: parse_flag(args, "--auth-token-key"),
                    compartment_id: parse_usize_flag(args, "--compartment-id"),
                },
                chain_id: require_u64_flag(
                    args,
                    "--chain-id",
                    "sigillum api profiles evm upsert --name <NAME> --rpc-url <URL> --chain-id <N>",
                ),
                max_priority_fee_per_gas_hex: parse_flag(args, "--max-priority-fee-per-gas-hex"),
                max_fee_per_gas_hex: parse_flag(args, "--max-fee-per-gas-hex"),
                native_gas_limit: parse_u64_flag(args, "--native-gas-limit"),
                erc20_gas_limit: parse_u64_flag(args, "--erc20-gas-limit"),
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
            const CREATE_USAGE: &str = "sigillum api profiles eth-seed create --name <NAME> --provider-profile <PROFILE> [--word-count 12|24] [--label <LABEL>] [--project-account <N>] [--compartment-id <N>] [--chain-id <N>] [--default-destination-address <ADDR>] [--mnemonic-passphrase-env VAR|--mnemonic-passphrase-stdin]";
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
            run_api_command(args, true, move |client| async move {
                client.create_eth_seed_wallet_profile(request).await
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

/// Dispatch `sigillum api plans <list|generate|approve|simulate|export>`.
fn cmd_api_plans(args: &[String]) {
    if args.len() < 2 {
        eprintln!("Usage: sigillum api plans <list|generate|approve|simulate|export> [...]");
        process::exit(1);
    }

    match args[1].as_str() {
        "list" => run_api_command(args, true, |client| async move {
            client.list_consolidation_plans().await
        }),
        "generate" => {
            let party_destinations = parse_multi_flag(args, "--party-destination")
                .into_iter()
                .map(|value| match value.split_once('=') {
                    Some((counterparty_id, destination_address)) => PartyDestination {
                        counterparty_id: counterparty_id.to_string(),
                        destination_address: destination_address.to_string(),
                    },
                    None => {
                        eprintln!(
                            "Usage: sigillum api plans generate --party-destination <counterparty_id>=<address>"
                        );
                        process::exit(1);
                    }
                })
                .collect();
            let request = ConsolidationPlanGenerateRequest {
                destination_address: parse_flag(args, "--destination-address"),
                wallet_family: parse_flag(args, "--wallet-family"),
                wallet_profile: parse_flag(args, "--wallet-profile"),
                provider_profile: parse_flag(args, "--provider-profile"),
                chain_id: parse_u64_flag(args, "--chain-id"),
                include_watch_only: flag_option(args, "--include-watch-only"),
                auto_queue_low_risk: flag_option(args, "--auto-queue-low-risk"),
                routing_strategy: parse_flag(args, "--routing-strategy"),
                party_destinations,
            };
            run_api_command(args, true, move |client| async move {
                client.generate_consolidation_plan(request).await
            });
        }
        "approve" => {
            let request = ConsolidationPlanApproveRequest {
                plan_id: require_flag(
                    args,
                    "--plan-id",
                    "sigillum api plans approve --plan-id <ID>",
                ),
                step_ids: parse_multi_flag(args, "--step-id"),
            };
            run_api_command(args, true, move |client| async move {
                client.approve_consolidation_plan(request).await
            });
        }
        "simulate" => {
            let request = ConsolidationPlanSimulateRequest {
                plan_id: require_flag(
                    args,
                    "--plan-id",
                    "sigillum api plans simulate --plan-id <ID>",
                ),
                step_ids: parse_multi_flag(args, "--step-id"),
            };
            run_api_command(args, true, move |client| async move {
                client.simulate_consolidation_plan(request).await
            });
        }
        "export" => {
            let request = ConsolidationPlanExportRequest {
                plan_id: require_flag(
                    args,
                    "--plan-id",
                    "sigillum api plans export --plan-id <ID> [--format call_manifest|safe_tx_builder] [--safe-address 0x...]",
                ),
                step_ids: parse_multi_flag(args, "--step-id"),
                format: parse_flag(args, "--format"),
                safe_address: parse_flag(args, "--safe-address"),
            };
            run_api_command(args, true, move |client| async move {
                client.export_consolidation_plan(request).await
            });
        }
        _ => {
            eprintln!("Usage: sigillum api plans <list|generate|approve|simulate|export> [...]");
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
    let client = build_client(args, require_session);
    let runtime = tokio::runtime::Runtime::new().unwrap_or_else(|error| {
        eprintln!("Failed to start async runtime: {error}");
        process::exit(1);
    });
    let response = match runtime.block_on(f(client)) {
        Ok(response) => response,
        Err(error) => report_client_error(error),
    };
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
  profiles eth-seed <list|create|delete> [...]  (create generates a new BIP-39 mnemonic and prints it exactly once)
  deposits <list|create-native|create-erc20|scan-announcements|refresh|enqueue-sweep|delete> [...]
  evm <nonce|balance|erc20-balance|fees> [...]  (read-only; no broadcast)
  chains <list|upsert|delete> [...]
  inventory <list|chains|watch|scan-evm> [...]  (scan supports --watch-address, --watch-address-file, --include-watch-book, --derivation-pattern, --account-limit)
  discovery <jobs|scan-evm> [...]
  risk <list|catalog|catalog-upsert|catalog-delete> [...]
  plans <list|generate|approve|simulate|export> [...]
  receiving <overview|refresh-balances|tag-deposit> [...]
  treasury <overview|policy|policy-update|receive-list|receive-allocate|receive-rotate|parties> [...]
  queue <list|process> [...]
  transit <encrypt|decrypt|hmac> [...]
  wallets <xpub-export|xpub-derive|stealth-export|stealth-generate|stealth-check> [...]  (read/derive only; no sign/send)
  maintenance run [...]

GLOBAL FLAGS:
  --url <BASE_URL>        Override daemon URL (default: SIGILLUM_BASE_URL or http://127.0.0.1:9743)
  --session <TOKEN>       Override daemon session token (default: SIGILLUM_SESSION_TOKEN)"
    );
}
