//! Consolidation plan daemon API commands.

use std::process;

use sigillum_api::request::{
    ConsolidationPlanApproveRequest, ConsolidationPlanExportRequest,
    ConsolidationPlanGenerateRequest, ConsolidationPlanSimulateRequest, PartyDestination,
    PlanEnqueuePlanRequest, PlanEnqueueStepRequest,
};

use super::{
    flag_option, has_flag, parse_flag, parse_multi_flag, parse_u64_flag, require_flag,
    run_api_command,
};

const PLANS_USAGE: &str =
    "sigillum api plans <list|generate|approve|simulate|export|enqueue-step|enqueue-plan> [...]";

/// Dispatch `sigillum api plans
/// <list|generate|approve|simulate|export|enqueue-step|enqueue-plan>`.
pub(super) fn cmd_api_plans(args: &[String]) {
    if args.len() < 2 {
        eprintln!("Usage: {PLANS_USAGE}");
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
        "enqueue-step" => {
            let request = PlanEnqueueStepRequest {
                plan_id: require_flag(
                    args,
                    "--plan-id",
                    "sigillum api plans enqueue-step --plan-id <ID> --step-id <ID> --confirm",
                ),
                step_id: require_flag(
                    args,
                    "--step-id",
                    "sigillum api plans enqueue-step --plan-id <ID> --step-id <ID> --confirm",
                ),
                // Sent as-is; the daemon refuses unless --confirm was given.
                confirm: has_flag(args, "--confirm"),
            };
            run_api_command(args, true, move |client| async move {
                client.enqueue_plan_step(request).await
            });
        }
        "enqueue-plan" => {
            // The exact expected phrase ("EXECUTE {n} PLAN STEPS TOTAL {wei}
            // WEI") is computed by the daemon from the currently eligible
            // steps; on mismatch the error message renders it verbatim, so
            // running without --confirmation shows the phrase to type.
            let request = PlanEnqueuePlanRequest {
                plan_id: require_flag(
                    args,
                    "--plan-id",
                    "sigillum api plans enqueue-plan --plan-id <ID> --confirmation \"<PHRASE>\"",
                ),
                confirmation: parse_flag(args, "--confirmation").unwrap_or_default(),
            };
            run_api_command(args, true, move |client| async move {
                client.enqueue_plan(request).await
            });
        }
        _ => {
            eprintln!("Usage: {PLANS_USAGE}");
            process::exit(1);
        }
    }
}
