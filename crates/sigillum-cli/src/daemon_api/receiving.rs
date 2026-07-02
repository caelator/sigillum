//! Receiving daemon API commands.

use std::process;

use sigillum_api::request::ReceivingDepositTagRequest;

use super::{has_flag, parse_flag, require_flag, run_api_command};

const USAGE: &str = "sigillum api receiving <overview|refresh-balances|tag-deposit> [--deposit-id <ID> [--counterparty-id <ID> | --clear]]";

pub(super) fn cmd_api_receiving(args: &[String]) {
    if args.len() < 2 {
        eprintln!("Usage: {USAGE}");
        process::exit(1);
    }

    match args[1].as_str() {
        "overview" => run_api_command(args, true, |client| async move {
            client.receiving_overview().await
        }),
        "refresh-balances" => run_api_command(args, true, |client| async move {
            client.refresh_receiving_balances().await
        }),
        "tag-deposit" => {
            let request = ReceivingDepositTagRequest {
                deposit_id: require_flag(args, "--deposit-id", USAGE),
                counterparty_id: if has_flag(args, "--clear") {
                    None
                } else {
                    parse_flag(args, "--counterparty-id")
                },
            };
            run_api_command(args, true, move |client| async move {
                client.tag_stealth_deposit(request).await
            });
        }
        _ => {
            eprintln!("Usage: {USAGE}");
            process::exit(1);
        }
    }
}
