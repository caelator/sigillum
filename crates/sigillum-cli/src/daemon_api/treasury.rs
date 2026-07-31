//! Treasury daemon API commands.

use std::process;

use sigillum_api::request::{
    CounterpartyCreateRequest, CounterpartyDeleteRequest, CounterpartyUpdateRequest,
    TreasuryAllowedDestinationInput, TreasuryPolicyUpdateRequest, TreasuryReceiveAllocateRequest,
    TreasuryReceivePurgeRequest, TreasuryReceiveRotateRequest,
};

use super::{
    bool_switch, parse_flag, parse_multi_flag, parse_u64_flag, require_flag, run_api_command,
};

/// Dispatch `sigillum api treasury
/// <overview|policy|policy-update|receive-list|receive-allocate|receive-rotate|receive-purge>`.
pub(super) fn cmd_api_treasury(args: &[String]) {
    const POLICY_UPDATE_USAGE: &str = "sigillum api treasury policy-update <--enabled|--disabled> \
        [--destination 0xADDR[:label]]... [--max-step-wei-hex 0x..] [--max-plan-wei-hex 0x..] \
        [--require-simulation|--no-require-simulation] [--block-cross-party-linkage <true|false>] \
        [--allow-claim-execution <true|false>] [--allow-gas-topups <true|false>] \
        [--allow-plan-execution <true|false>] [--allow-sweep-execution <true|false>] \
        [--allow-revoke-execution <true|false>] [--allow-exit-execution <true|false>] \
        [--execution-paused <true|false>] [--max-gas-topup-wei-hex 0x..] \
        [--max-fee-per-gas-cap-hex 0x..] \
        [--simulation-freshness-secs <SECS>] [--hot-floor-wei-hex 0x..] \
        [--hot-target-wei-hex 0x..] [--hot-overflow-wei-hex 0x..] \
        [--allow-treasury-automation <true|false>]";
    const PARTIES_USAGE: &str = "sigillum api treasury parties <list|create|update|delete> \
        [--id <ID>] [--name <NAME>] [--note <NOTE>] [--sweep-destination <ADDRESS>]";
    const RECEIVE_ALLOCATE_USAGE: &str = "sigillum api treasury receive-allocate \
        --wallet-profile <PROFILE> --purpose <PURPOSE> [--label <LABEL>] \
        [--one-time --sweep-destination <ADDRESS> [--min-sweep-wei-hex 0x..] [--purge-after-sweep]]";
    const TREASURY_USAGE: &str = "sigillum api treasury \
        <overview|policy|policy-update|receive-list|receive-allocate|receive-rotate|receive-purge|parties>";
    if args.len() < 2 {
        eprintln!("Usage: {TREASURY_USAGE}");
        process::exit(1);
    }

    match args[1].as_str() {
        "overview" => run_api_command(args, true, |client| async move {
            client.treasury_overview().await
        }),
        "policy" => run_api_command(args, true, |client| async move {
            client.get_treasury_policy().await
        }),
        "policy-update" => {
            let enabled = bool_switch(args, "--enabled", "--disabled").unwrap_or_else(|| {
                eprintln!("Usage: {POLICY_UPDATE_USAGE}");
                process::exit(1);
            });
            // `0xADDR:label` splits on the first ':' so labels may contain
            // further colons.
            let allowed_destinations = parse_multi_flag(args, "--destination")
                .into_iter()
                .map(|value| match value.split_once(':') {
                    Some((address, label)) => TreasuryAllowedDestinationInput {
                        address: address.to_string(),
                        label: Some(label.to_string()),
                    },
                    None => TreasuryAllowedDestinationInput {
                        address: value,
                        label: None,
                    },
                })
                .collect();
            let block_cross_party_linkage =
                parse_bool_value_flag(args, "--block-cross-party-linkage", POLICY_UPDATE_USAGE);
            let allow_claim_execution =
                parse_bool_value_flag(args, "--allow-claim-execution", POLICY_UPDATE_USAGE);
            let allow_gas_topups =
                parse_bool_value_flag(args, "--allow-gas-topups", POLICY_UPDATE_USAGE);
            let allow_plan_execution =
                parse_bool_value_flag(args, "--allow-plan-execution", POLICY_UPDATE_USAGE);
            let allow_sweep_execution =
                parse_bool_value_flag(args, "--allow-sweep-execution", POLICY_UPDATE_USAGE);
            let allow_revoke_execution =
                parse_bool_value_flag(args, "--allow-revoke-execution", POLICY_UPDATE_USAGE);
            let allow_exit_execution =
                parse_bool_value_flag(args, "--allow-exit-execution", POLICY_UPDATE_USAGE);
            let execution_paused =
                parse_bool_value_flag(args, "--execution-paused", POLICY_UPDATE_USAGE);
            let allow_treasury_automation =
                parse_bool_value_flag(args, "--allow-treasury-automation", POLICY_UPDATE_USAGE);
            let request = TreasuryPolicyUpdateRequest {
                enabled,
                allowed_destinations,
                max_step_native_wei_hex: parse_flag(args, "--max-step-wei-hex"),
                max_plan_native_wei_hex: parse_flag(args, "--max-plan-wei-hex"),
                require_simulation: bool_switch(
                    args,
                    "--require-simulation",
                    "--no-require-simulation",
                ),
                allow_raw_digest_signing: bool_switch(
                    args,
                    "--allow-raw-digest-signing",
                    "--disallow-raw-digest-signing",
                ),
                block_cross_party_linkage,
                allow_claim_execution,
                allow_gas_topups,
                max_gas_topup_wei_hex: parse_flag(args, "--max-gas-topup-wei-hex"),
                allow_plan_execution,
                allow_sweep_execution,
                allow_revoke_execution,
                allow_exit_execution,
                execution_paused,
                max_fee_per_gas_cap_hex: parse_flag(args, "--max-fee-per-gas-cap-hex"),
                simulation_freshness_secs: parse_u64_flag(args, "--simulation-freshness-secs"),
                hot_floor_wei_hex: parse_flag(args, "--hot-floor-wei-hex"),
                hot_target_wei_hex: parse_flag(args, "--hot-target-wei-hex"),
                hot_overflow_wei_hex: parse_flag(args, "--hot-overflow-wei-hex"),
                allow_treasury_automation,
            };
            run_api_command(args, true, move |client| async move {
                client.update_treasury_policy(request).await
            });
        }
        "parties" => cmd_api_treasury_parties(args, PARTIES_USAGE),
        "receive-list" => run_api_command(args, true, |client| async move {
            client.list_treasury_receive_allocations().await
        }),
        "receive-allocate" => {
            // Plan task 3.3 one-time mode: --one-time attaches the
            // auto-watch → auto-sweep → retire lifecycle; the destination is
            // required by the daemon in that mode.
            let request = TreasuryReceiveAllocateRequest {
                wallet_profile: require_flag(args, "--wallet-profile", RECEIVE_ALLOCATE_USAGE),
                purpose: require_flag(args, "--purpose", RECEIVE_ALLOCATE_USAGE),
                label: parse_flag(args, "--label"),
                counterparty_id: parse_flag(args, "--counterparty-id"),
                one_time: bool_switch(args, "--one-time", "--no-one-time"),
                sweep_destination_address: parse_flag(args, "--sweep-destination"),
                min_sweep_amount_hex: parse_flag(args, "--min-sweep-wei-hex"),
                purge_after_sweep: bool_switch(
                    args,
                    "--purge-after-sweep",
                    "--no-purge-after-sweep",
                ),
            };
            run_api_command(args, true, move |client| async move {
                client.allocate_treasury_receive_address(request).await
            });
        }
        "receive-rotate" => {
            let request = TreasuryReceiveRotateRequest {
                allocation_id: require_flag(
                    args,
                    "--allocation-id",
                    "sigillum api treasury receive-rotate --allocation-id <ID>",
                ),
            };
            run_api_command(args, true, move |client| async move {
                client.rotate_treasury_receive_address(request).await
            });
        }
        "receive-purge" => {
            let request = TreasuryReceivePurgeRequest {
                allocation_id: require_flag(
                    args,
                    "--allocation-id",
                    "sigillum api treasury receive-purge --allocation-id <ID>",
                ),
            };
            run_api_command(args, true, move |client| async move {
                client.purge_treasury_receive_address(request).await
            });
        }
        _ => {
            eprintln!("Usage: {TREASURY_USAGE}");
            process::exit(1);
        }
    }
}

fn parse_bool_value_flag(args: &[String], name: &str, usage: &str) -> Option<bool> {
    parse_flag(args, name).map(|value| match value.as_str() {
        "true" => true,
        "false" => false,
        _ => {
            eprintln!("Usage: {usage}");
            process::exit(1);
        }
    })
}

fn cmd_api_treasury_parties(args: &[String], usage: &str) {
    if args.len() < 3 {
        eprintln!("Usage: {usage}");
        process::exit(1);
    }

    match args[2].as_str() {
        "list" => run_api_command(
            args,
            true,
            |client| async move { client.list_parties().await },
        ),
        "create" => {
            let request = CounterpartyCreateRequest {
                name: require_flag(args, "--name", usage),
                note: parse_flag(args, "--note"),
                sweep_destination_address: parse_flag(args, "--sweep-destination"),
            };
            run_api_command(args, true, move |client| async move {
                client.create_party(request).await
            });
        }
        "update" => {
            let request = CounterpartyUpdateRequest {
                id: require_flag(args, "--id", usage),
                name: require_flag(args, "--name", usage),
                note: parse_flag(args, "--note"),
                sweep_destination_address: parse_flag(args, "--sweep-destination"),
            };
            run_api_command(args, true, move |client| async move {
                client.update_party(request).await
            });
        }
        "delete" => {
            let request = CounterpartyDeleteRequest {
                id: require_flag(args, "--id", usage),
            };
            run_api_command(args, true, move |client| async move {
                client.delete_party(request).await
            });
        }
        _ => {
            eprintln!("Usage: {usage}");
            process::exit(1);
        }
    }
}
