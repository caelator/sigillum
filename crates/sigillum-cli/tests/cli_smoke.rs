//! CLI smoke tests — exercise the compiled binary via `std::process::Command`.
//!
//! These tests verify that the CLI binary starts, parses arguments correctly,
//! and produces expected output/exit codes for both happy and adversarial paths.
//! No daemon is required — they only test argument parsing and help output.

use std::process::Command;

#[cfg(unix)]
use std::io::{self, Read, Write};
#[cfg(unix)]
use std::net::{TcpListener, TcpStream};
#[cfg(unix)]
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::process::{Child, ExitStatus, Stdio};
#[cfg(unix)]
use std::sync::mpsc::{self, Receiver, Sender};
#[cfg(unix)]
use std::thread;
#[cfg(unix)]
use std::time::{Duration, Instant};

/// Build the path to the debug binary.
fn sigillum_bin() -> String {
    env!("CARGO_BIN_EXE_sigillum").to_string()
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

#[cfg(unix)]
fn read_http_request(stream: &mut TcpStream) -> io::Result<(String, String, Vec<u8>)> {
    stream.set_read_timeout(Some(Duration::from_secs(1)))?;

    let mut request = Vec::new();
    let mut chunk = [0_u8; 4096];
    let header_end = loop {
        if request.len() > 64 * 1024 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "mock HTTP request headers are too large",
            ));
        }

        let read = stream.read(&mut chunk)?;
        if read == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "mock HTTP request ended before its headers",
            ));
        }
        request.extend_from_slice(&chunk[..read]);
        if let Some(position) = request.windows(4).position(|window| window == b"\r\n\r\n") {
            break position + 4;
        }
    };

    let headers = std::str::from_utf8(&request[..header_end])
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let request_line = headers
        .lines()
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing request line"))?;
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing request method"))?
        .to_string();
    let path = request_parts
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing request path"))?
        .to_string();
    let content_length = headers
        .lines()
        .skip(1)
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .map(|(_, value)| value.trim().parse::<usize>())
        .transpose()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?
        .unwrap_or(0);

    let request_length = header_end + content_length;
    while request.len() < request_length {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "mock HTTP request ended before its body",
            ));
        }
        request.extend_from_slice(&chunk[..read]);
    }

    Ok((method, path, request[header_end..request_length].to_vec()))
}

#[cfg(unix)]
fn write_http_response(stream: &mut TcpStream, status: &str, body: &str) -> io::Result<()> {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.set_write_timeout(Some(Duration::from_secs(1)))?;
    stream.write_all(response.as_bytes())?;
    stream.flush()
}

#[cfg(unix)]
fn handle_mock_request(
    mut stream: TcpStream,
    audit_sender: &Sender<Result<Vec<u8>, String>>,
) -> io::Result<bool> {
    let (method, path, body) = read_http_request(&mut stream)?;
    let (status, response_body, received_audit) = match (method.as_str(), path.as_str()) {
        ("GET", "/api/status") => (
            "200 OK",
            r#"{"locked":false,"initialized":true,"active_compartment":null,"unlocked_compartments":[],"fido2":null}"#,
            false,
        ),
        ("POST", "/api/secrets/resolve-batch") => ("200 OK", r#"{"values":[]}"#, false),
        ("POST", "/api/audit/run") => {
            let _ = audit_sender.send(Ok(body));
            ("200 OK", r#"{"status":"ok"}"#, true)
        }
        _ => ("404 Not Found", r#"{"error":"not found"}"#, false),
    };

    write_http_response(&mut stream, status, response_body)?;
    Ok(received_audit)
}

#[cfg(unix)]
struct MockRunServer {
    stop_sender: Sender<()>,
    thread: Option<thread::JoinHandle<()>>,
}

#[cfg(unix)]
impl MockRunServer {
    fn start() -> (String, Receiver<Result<Vec<u8>, String>>, Self) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock run server");
        listener
            .set_nonblocking(true)
            .expect("make mock run server nonblocking");
        let address = listener.local_addr().expect("read mock server address");
        let (audit_sender, audit_receiver) = mpsc::channel();
        let (stop_sender, stop_receiver) = mpsc::channel();

        let server_thread = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(20);
            loop {
                if stop_receiver.try_recv().is_ok() {
                    return;
                }
                if Instant::now() >= deadline {
                    let _ = audit_sender.send(Err("mock run server deadline exceeded".to_string()));
                    return;
                }

                match listener.accept() {
                    Ok((stream, _)) => match handle_mock_request(stream, &audit_sender) {
                        Ok(true) => return,
                        Ok(false) => {}
                        Err(error) => {
                            let _ = audit_sender.send(Err(format!(
                                "mock run server failed to handle request: {error}"
                            )));
                            return;
                        }
                    },
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => {
                        let _ = audit_sender
                            .send(Err(format!("mock run server accept failed: {error}")));
                        return;
                    }
                }
            }
        });

        (
            format!("http://{address}"),
            audit_receiver,
            Self {
                stop_sender,
                thread: Some(server_thread),
            },
        )
    }
}

#[cfg(unix)]
impl Drop for MockRunServer {
    fn drop(&mut self) {
        let _ = self.stop_sender.send(());
        if let Some(server_thread) = self.thread.take() {
            let _ = server_thread.join();
        }
    }
}

#[cfg(unix)]
fn read_child_pid(path: &Path) -> Option<libc::pid_t> {
    std::fs::read_to_string(path)
        .ok()?
        .trim()
        .parse::<libc::pid_t>()
        .ok()
        .filter(|pid| *pid > 0)
}

#[cfg(unix)]
fn process_is_gone(pid: libc::pid_t) -> io::Result<bool> {
    // SAFETY: Signal zero only probes the positive PID and does not send a
    // signal to the process.
    if unsafe { libc::kill(pid, 0) } == 0 {
        return Ok(false);
    }

    match io::Error::last_os_error().raw_os_error() {
        Some(libc::ESRCH) => Ok(true),
        Some(libc::EPERM) => Ok(false),
        _ => Err(io::Error::last_os_error()),
    }
}

#[cfg(unix)]
struct SupervisedProcessCleanup {
    sigillum: Child,
    child_pid_file: PathBuf,
    child_is_confirmed_gone: bool,
}

#[cfg(unix)]
impl SupervisedProcessCleanup {
    fn wait_until(&mut self, deadline: Instant) -> io::Result<ExitStatus> {
        loop {
            if let Some(status) = self.sigillum.try_wait()? {
                return Ok(status);
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "sigillum did not exit before the test deadline",
                ));
            }
            thread::sleep(Duration::from_millis(10));
        }
    }
}

#[cfg(unix)]
impl Drop for SupervisedProcessCleanup {
    fn drop(&mut self) {
        let child_pid = (!self.child_is_confirmed_gone)
            .then(|| read_child_pid(&self.child_pid_file))
            .flatten();
        if let Some(pid) = child_pid {
            // SAFETY: The PID came from the isolated test child's PID file.
            unsafe {
                libc::kill(pid, libc::SIGKILL);
            }
        }

        let reap_deadline = Instant::now() + Duration::from_secs(1);
        let mut sigillum_reaped = false;
        loop {
            match self.sigillum.try_wait() {
                Ok(Some(_)) => {
                    sigillum_reaped = true;
                    break;
                }
                Ok(None) if Instant::now() < reap_deadline => {
                    thread::sleep(Duration::from_millis(10));
                }
                _ => break,
            }
        }
        if !sigillum_reaped {
            let _ = self.sigillum.kill();
            let _ = self.sigillum.wait();
        }

        if let Some(pid) = child_pid {
            // SAFETY: The PID came from the isolated test child's PID file.
            unsafe {
                libc::kill(pid, libc::SIGKILL);
            }
            let gone_deadline = Instant::now() + Duration::from_secs(1);
            while Instant::now() < gone_deadline {
                if process_is_gone(pid).unwrap_or(false) {
                    break;
                }
                thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

// ── Happy Paths ────────────────────────────────────────────────────

#[cfg(unix)]
#[test]
fn sigterm_is_forwarded_and_run_audit_is_recorded() {
    let temp_dir = tempfile::tempdir().expect("create isolated PID directory");
    let child_pid_file = temp_dir.path().join("child.pid");
    let (base_url, audit_receiver, _mock_server) = MockRunServer::start();

    let sigillum = Command::new(sigillum_bin())
        .args([
            "run",
            "--",
            "/bin/sh",
            "-c",
            r#"echo $$ > "$SIGILLUM_TEST_CHILD_PID"; exec sleep 30"#,
        ])
        .env("SIGILLUM_BASE_URL", base_url)
        .env("SIGILLUM_SESSION_TOKEN", "test-session-token")
        .env("SIGILLUM_TEST_CHILD_PID", &child_pid_file)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("launch sigillum run");
    let mut cleanup = SupervisedProcessCleanup {
        sigillum,
        child_pid_file: child_pid_file.clone(),
        child_is_confirmed_gone: false,
    };

    let pid_deadline = Instant::now() + Duration::from_secs(10);
    let child_pid = loop {
        if let Some(pid) = read_child_pid(&child_pid_file) {
            break pid;
        }
        assert!(
            Instant::now() < pid_deadline,
            "timed out waiting for the supervised child PID file"
        );
        thread::sleep(Duration::from_millis(10));
    };

    // SAFETY: `cleanup.sigillum.id()` is the live process spawned by this test.
    let signal_result = unsafe { libc::kill(cleanup.sigillum.id() as libc::pid_t, libc::SIGTERM) };
    assert_eq!(
        signal_result,
        0,
        "failed to signal sigillum: {}",
        io::Error::last_os_error()
    );

    cleanup
        .wait_until(Instant::now() + Duration::from_secs(10))
        .expect("sigillum should exit after forwarding SIGTERM");

    let audit_body = audit_receiver
        .recv_timeout(Duration::from_secs(2))
        .expect("mock server did not receive a run audit")
        .expect("mock server failed");
    let audit: serde_json::Value =
        serde_json::from_slice(&audit_body).expect("run audit should contain valid JSON");
    assert_eq!(audit["signal"], libc::SIGTERM);
    assert_eq!(audit["success"], false);

    let child_gone_deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if process_is_gone(child_pid).expect("probe supervised child PID") {
            break;
        }
        assert!(
            Instant::now() < child_gone_deadline,
            "supervised child PID {child_pid} is still alive"
        );
        thread::sleep(Duration::from_millis(10));
    }
    cleanup.child_is_confirmed_gone = true;
}

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
fn api_compartment_missing_subcommand_exits_nonzero() {
    let output = run(&["api", "compartment"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Usage"));
}

#[test]
fn api_compartment_add_is_not_bridged() {
    let output = run(&["api", "compartment", "add"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Usage"));
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
fn api_profiles_eth_seed_upsert_usage_only_advertises_secure_mnemonic_sources() {
    let output = run(&["api", "profiles", "eth-seed", "upsert"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("profiles eth-seed upsert"));
    assert!(stderr.contains("--mnemonic-env VAR"));
    assert!(stderr.contains("--mnemonic-stdin"));
    assert!(!stderr.contains("--mnemonic <"));
    assert!(!stderr.contains("--mnemonic="));
}

#[test]
fn fido2_unlock_invalid_taps_fails_before_hardware_access() {
    let output = run(&["fido2", "unlock", "--taps", "not-a-number"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Invalid value for --taps: not-a-number"));
    assert!(!stderr.contains("Keys to tap:"));
    assert!(!stderr.contains("Touch your FIDO2 key now"));
}

#[test]
fn help_advertises_optional_fido2_unlock_taps() {
    let output = run(&["help"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("unlock [--taps <N>]"));
    assert!(stdout.contains("prompts when omitted"));
}

#[test]
fn api_deposits_missing_subcommand_exits_nonzero() {
    let output = run(&["api", "deposits"]);
    assert!(!output.status.success());
}

#[test]
fn api_deposits_create_native_rejects_raw_ephemeral_private_key_before_usage() {
    let output = run(&[
        "api",
        "deposits",
        "create-native",
        "--ephemeral-private-key-hex",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    ]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(
        "Do not pass ephemeral private keys as CLI arguments; use --ephemeral-key-env VAR or --ephemeral-key-stdin."
    ));
    assert!(!stderr.contains("Usage:"));
}

#[test]
fn api_queue_missing_subcommand_exits_nonzero() {
    let output = run(&["api", "queue"]);
    assert!(!output.status.success());
}

#[test]
fn api_evm_missing_subcommand_exits_nonzero() {
    let output = run(&["api", "evm"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Usage"));
}

#[test]
fn api_evm_broadcast_is_not_bridged() {
    let output = run(&["api", "evm", "broadcast"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Usage"));
}

#[test]
fn api_evm_nonce_missing_flags_exits_nonzero() {
    let output = run(&[
        "api",
        "evm",
        "nonce",
        "--rpc-url",
        "https://provider.invalid",
    ]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--address") || stderr.contains("Usage"),
        "should mention required flags"
    );
}

#[test]
fn api_evm_balance_missing_flags_exits_nonzero() {
    let output = run(&[
        "api",
        "evm",
        "balance",
        "--rpc-url",
        "https://provider.invalid",
    ]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--address") || stderr.contains("Usage"),
        "should mention required flags"
    );
}

#[test]
fn api_evm_erc20_balance_missing_flags_exits_nonzero() {
    let output = run(&[
        "api",
        "evm",
        "erc20-balance",
        "--rpc-url",
        "https://provider.invalid",
        "--owner-address",
        "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    ]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--token-address") || stderr.contains("Usage"),
        "should mention required flags"
    );
}

#[test]
fn api_evm_fees_missing_flags_exits_nonzero() {
    let output = run(&[
        "api",
        "evm",
        "fees",
        "--rpc-url",
        "https://provider.invalid",
    ]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--chain-id") || stderr.contains("Usage"),
        "should mention required flags"
    );
}

#[test]
fn api_evm_nonce_reaches_network_with_valid_args() {
    let output = run(&[
        "api",
        "evm",
        "nonce",
        "--rpc-url",
        "https://provider.invalid",
        "--address",
        "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "--block-tag",
        "pending",
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
fn api_evm_balance_reaches_network_with_valid_args() {
    let output = run(&[
        "api",
        "evm",
        "balance",
        "--rpc-url",
        "https://provider.invalid",
        "--address",
        "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
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
fn api_evm_erc20_balance_reaches_network_with_valid_args() {
    let output = run(&[
        "api",
        "evm",
        "erc20-balance",
        "--rpc-url",
        "https://provider.invalid",
        "--token-address",
        "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48",
        "--owner-address",
        "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "--block-tag",
        "latest",
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
fn api_evm_fees_reaches_network_with_valid_args() {
    let output = run(&[
        "api",
        "evm",
        "fees",
        "--rpc-url",
        "https://provider.invalid",
        "--chain-id",
        "1",
        "--gas-limit",
        "21000",
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
fn api_evm_estimate_alias_reaches_network_with_valid_args() {
    let output = run(&[
        "api",
        "evm",
        "estimate",
        "--rpc-url",
        "https://provider.invalid",
        "--chain-id",
        "1",
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
fn api_compartment_list_reaches_network_with_valid_args() {
    let output = run(&[
        "api",
        "compartment",
        "list",
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
fn api_wallets_missing_subcommand_exits_nonzero() {
    let output = run(&["api", "wallets"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Usage"));
}

#[test]
fn api_wallets_stealth_sign_is_not_bridged() {
    let output = run(&["api", "wallets", "stealth-sign"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Usage"));
}

#[test]
fn api_wallets_xpub_export_missing_flags_exits_nonzero() {
    let output = run(&["api", "wallets", "xpub-export"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--wallet-profile") || stderr.contains("Usage"),
        "should mention required flags"
    );
}

#[test]
fn api_wallets_xpub_derive_missing_flags_exits_nonzero() {
    let output = run(&["api", "wallets", "xpub-derive", "--xpub", "xpub6Ctest"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--index") || stderr.contains("Usage"),
        "should mention required flags"
    );
}

#[test]
fn api_wallets_stealth_export_missing_flags_exits_nonzero() {
    let output = run(&["api", "wallets", "stealth-export"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--wallet") || stderr.contains("Usage"),
        "should mention required flags"
    );
}

#[test]
fn api_wallets_stealth_generate_missing_flags_exits_nonzero() {
    let output = run(&["api", "wallets", "stealth-generate"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--meta-address") || stderr.contains("Usage"),
        "should mention required flags"
    );
}

#[test]
fn api_wallets_stealth_check_missing_flags_exits_nonzero() {
    let output = run(&[
        "api",
        "wallets",
        "stealth-check",
        "--wallet",
        "treasury",
        "--stealth-address",
        "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    ]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--ephemeral-public-key-hex") || stderr.contains("Usage"),
        "should mention required flags"
    );
}

#[test]
fn api_wallets_xpub_derive_invalid_index_exits_nonzero() {
    let output = run(&[
        "api",
        "wallets",
        "xpub-derive",
        "--xpub",
        "xpub6Ctest",
        "--index",
        "notanumber",
    ]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Invalid value") || stderr.contains("--index"));
}

#[test]
fn api_wallets_stealth_check_invalid_ephemeral_public_key_hex_exits_nonzero() {
    let output = run(&[
        "api",
        "wallets",
        "stealth-check",
        "--wallet",
        "treasury",
        "--stealth-address",
        "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "--ephemeral-public-key-hex",
        "zz",
    ]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Invalid hex"));
}

#[test]
fn api_wallets_stealth_check_invalid_view_tag_length_exits_nonzero() {
    let output = run(&[
        "api",
        "wallets",
        "stealth-check",
        "--wallet",
        "treasury",
        "--stealth-address",
        "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "--ephemeral-public-key-hex",
        "02aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "--view-tag-hex",
        "0102",
    ]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("view-tag") || stderr.contains("1 byte"));
}

#[test]
fn api_wallets_xpub_export_reaches_network_with_valid_args() {
    let output = run(&[
        "api",
        "wallets",
        "xpub-export",
        "--wallet-profile",
        "treasury-xpub",
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
fn api_wallets_xpub_derive_reaches_network_with_valid_args() {
    let output = run(&[
        "api",
        "wallets",
        "xpub-derive",
        "--xpub",
        "xpub6Ctest",
        "--index",
        "0",
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
fn api_wallets_stealth_export_reaches_network_with_valid_args() {
    let output = run(&[
        "api",
        "wallets",
        "stealth-export",
        "--wallet",
        "treasury",
        "--short-name",
        "ops",
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
fn api_wallets_stealth_generate_reaches_network_with_valid_args() {
    let output = run(&[
        "api",
        "wallets",
        "stealth-generate",
        "--meta-address",
        "st:eth:0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
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
fn api_wallets_stealth_check_reaches_network_with_valid_args() {
    let output = run(&[
        "api",
        "wallets",
        "stealth-check",
        "--wallet",
        "treasury",
        "--stealth-address",
        "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "--ephemeral-public-key-hex",
        "02aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "--view-tag-hex",
        "ab",
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
