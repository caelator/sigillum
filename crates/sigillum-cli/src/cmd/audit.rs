use std::process;

use chrono::{DateTime, Utc};
use sigillum_client::{AuditEventQuery, SigillumClient};

use crate::daemon_api::{daemon_base_url, ensure_daemon_ready, require_session_token};

pub fn cmd_audit(args: &[String]) {
    let base_url = daemon_base_url(args);
    if let Err(error) = ensure_daemon_ready(&base_url) {
        eprintln!("failed to reach daemon: {error}");
        process::exit(1);
    }

    let session_token = require_session_token(args);
    let query = parse_query(args);
    let client = SigillumClient::new(base_url);
    client.set_session_token(session_token);

    let runtime = tokio::runtime::Runtime::new().unwrap_or_else(|error| {
        eprintln!("failed to start async runtime: {error}");
        process::exit(1);
    });

    if args.first().is_some_and(|arg| arg == "verify") {
        let scope = args
            .get(1)
            .filter(|value| !value.starts_with("--"))
            .map(String::as_str);
        let report = runtime
            .block_on(client.audit_verify(scope))
            .unwrap_or_else(|error| {
                eprintln!("failed to verify audit chain: {error}");
                process::exit(1);
            });
        println!(
            "scope={} status={} verified={} broken={} legacy={}",
            report.scope, report.status, report.verified, report.broken, report.legacy
        );
        return;
    }

    let events = runtime
        .block_on(client.audit_events_query(query))
        .unwrap_or_else(|error| {
            eprintln!("failed to query audit events: {error}");
            process::exit(1);
        });

    if events.is_empty() {
        println!("No audit events matched.");
        return;
    }

    for event in events {
        let timestamp = DateTime::<Utc>::from_timestamp(event.created_at_unix as i64, 0)
            .map(|value| value.to_rfc3339())
            .unwrap_or_else(|| event.created_at_unix.to_string());
        let compartment = event
            .compartment_id
            .map(|value| format!(" compartment={value}"))
            .unwrap_or_default();
        let details = if event.details.is_null() || event.details == serde_json::json!({}) {
            String::new()
        } else {
            format!(
                " {}",
                serde_json::to_string(&event.details).unwrap_or_else(|_| "{}".into())
            )
        };
        println!("{timestamp} {}{compartment}{details}", event.kind);
    }
}

fn parse_query(args: &[String]) -> AuditEventQuery {
    AuditEventQuery {
        tail: Some(parse_usize_flag(args, "--tail").unwrap_or(50)),
        kind: parse_flag(args, "--kind"),
        since: parse_since_flag(args, "--since"),
        key: parse_flag(args, "--key"),
    }
}

fn parse_since_flag(args: &[String], flag: &str) -> Option<u64> {
    let raw = parse_flag(args, flag)?;
    raw.parse::<u64>()
        .ok()
        .or_else(|| {
            chrono::DateTime::parse_from_rfc3339(&raw)
                .ok()
                .map(|value| value.timestamp().max(0) as u64)
        })
        .or_else(|| {
            eprintln!("invalid value for {flag}: expected unix seconds or RFC3339 timestamp");
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

fn parse_usize_flag(args: &[String], flag: &str) -> Option<usize> {
    parse_flag(args, flag).map(|raw| {
        raw.parse::<usize>().unwrap_or_else(|_| {
            eprintln!("invalid value for {flag}: {raw}");
            process::exit(1);
        })
    })
}
