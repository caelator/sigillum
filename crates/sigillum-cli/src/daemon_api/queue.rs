//! Queue daemon API commands.

use std::process;

use sigillum_api::request::QueueProcessRequest;

use super::{parse_flag, parse_usize_flag, run_api_command};

/// Dispatch `sigillum api queue <list|process|pause|resume>`.
pub(super) fn cmd_api_queue(args: &[String]) {
    if args.len() < 2 {
        eprintln!("Usage: sigillum api queue <list|process|pause|resume> [...]");
        process::exit(1);
    }

    match args[1].as_str() {
        "list" => run_api_command(args, true, |client| async move {
            client.list_queue_jobs().await
        }),
        "process" => {
            let request = QueueProcessRequest {
                id: parse_flag(args, "--id"),
                limit: parse_usize_flag(args, "--limit"),
            };
            run_api_command(args, true, move |client| async move {
                client.process_queue(request).await
            });
        }
        "pause" => run_api_command(
            args,
            true,
            |client| async move { client.pause_queue().await },
        ),
        "resume" => run_api_command(
            args,
            true,
            |client| async move { client.resume_queue().await },
        ),
        _ => {
            eprintln!("Usage: sigillum api queue <list|process|pause|resume> [...]");
            process::exit(1);
        }
    }
}
