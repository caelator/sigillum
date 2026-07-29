//! `sigillum doctor` — local production-readiness checks.

use std::path::Path;
use std::process;
use std::time::Duration;

use sigillum_client::SigillumClient;
use url::Url;

use crate::base_dir;
use crate::daemon_api::{daemon_base_url, session_token_from_args};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CheckLevel {
    Ok,
    Warn,
    Fail,
    Info,
}

impl CheckLevel {
    fn label(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Warn => "warn",
            Self::Fail => "fail",
            Self::Info => "info",
        }
    }
}

#[derive(Default)]
struct DoctorReport {
    failures: usize,
}

impl DoctorReport {
    fn check(&mut self, level: CheckLevel, name: &str, detail: impl AsRef<str>) {
        if level == CheckLevel::Fail {
            self.failures += 1;
        }
        println!("[{}] {name}: {}", level.label(), detail.as_ref());
    }
}

pub fn cmd_doctor(args: &[String]) {
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print_usage();
        return;
    }

    let base = base_dir();
    let daemon_url = daemon_base_url(args);
    let session_token = session_token_from_args(args);
    let mut report = DoctorReport::default();

    println!("Sigillum doctor");
    println!("Production boundary: local-first single-host sidecar only");
    println!();

    check_data_dir(&mut report, &base);
    check_audit_db(&mut report, &base);
    check_daemon_url(&mut report, &daemon_url);
    check_session(&mut report, session_token.as_deref());
    check_daemon(&mut report, &daemon_url, session_token.as_deref());
    check_env(&mut report);

    println!();
    if report.failures == 0 {
        println!("doctor: no blocking local-readiness failures found");
    } else {
        eprintln!("doctor: {} blocking check(s) failed", report.failures);
        process::exit(1);
    }
}

fn print_usage() {
    println!("Usage: sigillum doctor [--url URL] [--session TOKEN]");
}

fn check_data_dir(report: &mut DoctorReport, base: &Path) {
    if !base.exists() {
        report.check(
            CheckLevel::Warn,
            "data dir",
            format!("{} does not exist yet", base.display()),
        );
        return;
    }

    if !base.is_dir() {
        report.check(
            CheckLevel::Fail,
            "data dir",
            format!("{} exists but is not a directory", base.display()),
        );
        return;
    }

    report.check(CheckLevel::Ok, "data dir", base.display().to_string());

    match std::fs::metadata(base) {
        Ok(metadata) => check_data_dir_permissions(report, base, &metadata),
        Err(error) => report.check(
            CheckLevel::Fail,
            "data dir permissions",
            format!("cannot inspect {}: {error}", base.display()),
        ),
    }
}

#[cfg(unix)]
fn check_data_dir_permissions(
    report: &mut DoctorReport,
    base: &Path,
    metadata: &std::fs::Metadata,
) {
    use std::os::unix::fs::PermissionsExt;

    let mode = metadata.permissions().mode() & 0o777;
    if mode & 0o077 == 0 {
        report.check(CheckLevel::Ok, "data dir permissions", format!("{mode:o}"));
    } else {
        report.check(
            CheckLevel::Fail,
            "data dir permissions",
            format!(
                "{} is {mode:o}; run chmod 700 {}",
                base.display(),
                base.display()
            ),
        );
    }
}

#[cfg(not(unix))]
fn check_data_dir_permissions(
    report: &mut DoctorReport,
    _base: &Path,
    _metadata: &std::fs::Metadata,
) {
    report.check(
        CheckLevel::Info,
        "data dir permissions",
        "manual ACL review required on this platform",
    );
}

fn check_audit_db(report: &mut DoctorReport, base: &Path) {
    let audit_db = base.join("audit.db");
    if !audit_db.exists() {
        report.check(
            CheckLevel::Warn,
            "audit DB",
            format!("{} has not been created yet", audit_db.display()),
        );
        return;
    }

    match rusqlite::Connection::open_with_flags(
        &audit_db,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    ) {
        Ok(_) => report.check(CheckLevel::Ok, "audit DB", audit_db.display().to_string()),
        Err(error) => report.check(
            CheckLevel::Fail,
            "audit DB",
            format!("{} is not readable SQLite: {error}", audit_db.display()),
        ),
    }
}

fn check_daemon_url(report: &mut DoctorReport, daemon_url: &str) {
    match Url::parse(daemon_url) {
        Ok(url) => {
            let host = url.host_str().unwrap_or_default();
            if host == "127.0.0.1" || host == "localhost" || host == "::1" {
                report.check(CheckLevel::Ok, "daemon URL", daemon_url);
            } else {
                report.check(
                    CheckLevel::Fail,
                    "daemon URL",
                    format!("{daemon_url} is not local; remote daemon operation is unsupported"),
                );
            }
        }
        Err(error) => report.check(
            CheckLevel::Fail,
            "daemon URL",
            format!("{daemon_url} is invalid: {error}"),
        ),
    }
}

fn check_session(report: &mut DoctorReport, session_token: Option<&str>) {
    if session_token
        .map(|token| !token.trim().is_empty())
        .unwrap_or(false)
    {
        report.check(CheckLevel::Ok, "session token", "present");
    } else {
        report.check(
            CheckLevel::Warn,
            "session token",
            "not set; authenticated daemon commands need --session or SIGILLUM_SESSION_TOKEN",
        );
    }
}

fn check_daemon(report: &mut DoctorReport, daemon_url: &str, session_token: Option<&str>) {
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(error) => {
            report.check(
                CheckLevel::Fail,
                "daemon reachability",
                format!("cannot start async runtime: {error}"),
            );
            return;
        }
    };

    let client = match SigillumClient::new(daemon_url.to_string()) {
        Ok(client) => client,
        Err(error) => {
            report.check(
                CheckLevel::Fail,
                "daemon reachability",
                format!("{daemon_url} returned an error: {error}"),
            );
            return;
        }
    };
    if let Some(token) = session_token {
        client.set_session_token(token);
    }

    let result = runtime
        .block_on(async { tokio::time::timeout(Duration::from_secs(3), client.status()).await });

    match result {
        Ok(Ok(status)) => {
            report.check(
                CheckLevel::Ok,
                "daemon reachability",
                "status endpoint responded",
            );
            let lock_state = if status.locked { "locked" } else { "unlocked" };
            report.check(
                CheckLevel::Info,
                "daemon state",
                format!(
                    "initialized={}, lock_state={}, unlocked_compartments={}",
                    status.initialized,
                    lock_state,
                    status.unlocked_compartments.len()
                ),
            );
            if status.active_compartment.is_none() {
                report.check(
                    CheckLevel::Warn,
                    "active compartment",
                    "none reported by daemon",
                );
            } else {
                report.check(CheckLevel::Ok, "active compartment", "reported by daemon");
            }
        }
        Ok(Err(error)) => report.check(
            CheckLevel::Fail,
            "daemon reachability",
            format!("{daemon_url} returned an error: {error}"),
        ),
        Err(_) => report.check(
            CheckLevel::Fail,
            "daemon reachability",
            format!("{daemon_url} did not respond within 3s"),
        ),
    }
}

fn check_env(report: &mut DoctorReport) {
    for name in [
        "SIGILLUM_BASE_DIR",
        "SIGILLUM_BASE_URL",
        "SIGILLUM_DAEMON_URL",
        "SIGILLUM_SESSION_TOKEN",
        "SIGILLUM_DAEMON_SESSION_TOKEN",
    ] {
        let label = if name == "SIGILLUM_DAEMON_SESSION_TOKEN" {
            "SIGILLUM_DAEMON_SESSION_TOKEN (gateway only)"
        } else {
            name
        };
        match std::env::var(name) {
            Ok(value) if name.contains("TOKEN") && !value.trim().is_empty() => {
                report.check(CheckLevel::Info, label, "set");
            }
            Ok(value) if value.trim().is_empty() => {
                report.check(CheckLevel::Warn, label, "set but empty");
            }
            Ok(value) => report.check(CheckLevel::Info, label, value),
            Err(_) => report.check(CheckLevel::Info, label, "not set"),
        }
    }
}
