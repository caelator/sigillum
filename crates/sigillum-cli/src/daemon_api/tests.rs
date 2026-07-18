use super::*;
use sigillum_api::response::EthSeedWalletProfile;

const TEST_MNEMONIC: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

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
            plan_mnemonic_output(&args(&["--mnemonic-out", "/tmp/seed.txt"]), stdout_tty).unwrap();
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

// ── Client error rendering ───────────────────────────────────

#[test]
fn format_client_error_includes_code_and_field_errors() {
    let error = ClientError::Api {
        status: reqwest::StatusCode::FORBIDDEN,
        message: "execution gates deny this operation".into(),
        code: Some("execution_gate_denied".into()),
        fields: Vec::new(),
    };
    assert_eq!(
        format_client_error(&error),
        "error[execution_gate_denied]: execution gates deny this operation"
    );

    let error = ClientError::Api {
        status: reqwest::StatusCode::BAD_REQUEST,
        message: "name exceeds maximum length of 256 bytes".into(),
        code: Some("validation_failed".into()),
        fields: vec![
            sigillum_client::FieldError {
                field: "name".into(),
                message: "name exceeds maximum length of 256 bytes".into(),
            },
            sigillum_client::FieldError {
                field: "rpc_url".into(),
                message: "rpc_url exceeds maximum length of 2048 bytes".into(),
            },
        ],
    };
    assert_eq!(
        format_client_error(&error),
        "error[validation_failed]: name exceeds maximum length of 256 bytes\n  \
         name: name exceeds maximum length of 256 bytes\n  \
         rpc_url: rpc_url exceeds maximum length of 2048 bytes"
    );
}

#[test]
fn format_client_error_without_code_keeps_legacy_rendering() {
    let error = ClientError::Api {
        status: reqwest::StatusCode::UNAUTHORIZED,
        message: "Invalid or missing session token.".into(),
        code: None,
        fields: Vec::new(),
    };
    assert_eq!(
        format_client_error(&error),
        "api error (401 Unauthorized): Invalid or missing session token."
    );

    let encoding = ClientError::Encoding("bad hex".into());
    assert_eq!(
        format_client_error(&encoding),
        "invalid response encoding: bad hex"
    );
}
