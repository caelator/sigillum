use super::*;
use tempfile::TempDir;

fn meta(id: usize, threshold: usize, label: &str) -> CompartmentMeta {
    CompartmentMeta {
        id,
        label: label.into(),
        threshold,
        passphrase_mode: None,
    }
}

#[test]
fn sessions_track_active_compartments_independently() {
    let dir = TempDir::new().unwrap();
    let state = AppState::new(dir.path().to_path_buf());

    state.unlock_compartment(0, [1u8; 32], meta(0, 1, "daily"));
    state.unlock_compartment(1, [2u8; 32], meta(1, 2, "secure"));

    let session_a = state.create_session(Some(0));
    let session_b = state.create_session(Some(0));

    state.switch_active_for(&session_a, 1).unwrap();

    assert_eq!(state.active_compartment_id_for(&session_a), Some(1));
    assert_eq!(state.active_compartment_id_for(&session_b), Some(0));
}

#[test]
fn removing_active_compartment_repoints_sessions() {
    let dir = TempDir::new().unwrap();
    let state = AppState::new(dir.path().to_path_buf());

    state.unlock_compartment(0, [1u8; 32], meta(0, 1, "daily"));
    state.unlock_compartment(1, [2u8; 32], meta(1, 2, "secure"));

    let session = state.create_session(Some(1));
    state.remove_compartment(1);

    assert_eq!(state.active_compartment_id_for(&session), Some(0));
}

#[test]
fn lock_all_clears_sessions_and_vault_instances() {
    let dir = TempDir::new().unwrap();
    let state = AppState::new(dir.path().to_path_buf());

    state.unlock_compartment(0, [1u8; 32], meta(0, 1, "daily"));
    let session = state.create_session(Some(0));

    assert!(state.verify_token(&session));
    assert!(state.with_vault(0, |_| true).is_some());

    state.lock_all();

    assert!(!state.verify_token(&session));
    assert!(state.with_vault(0, |_| true).is_none());
    assert!(!state.is_unlocked());
}

#[test]
fn idle_sessions_are_rejected_and_removed() {
    let dir = TempDir::new().unwrap();
    let state = AppState::new(dir.path().to_path_buf());

    state.unlock_compartment(0, [1u8; 32], meta(0, 1, "daily"));
    let session = state.create_session(Some(0));
    {
        let mut sessions = state.sessions.lock();
        let session = sessions.get_mut(&session).unwrap();
        session.last_activity =
            Instant::now() - Duration::from_secs(state.runtime_policy().idle_lock_secs + 1);
    }

    assert!(!state.verify_token(&session));
    assert_eq!(state.session_count(), 0);
}

#[test]
fn idle_lock_due_requires_all_sessions_to_be_idle() {
    let dir = TempDir::new().unwrap();
    let state = AppState::new(dir.path().to_path_buf());

    state.unlock_compartment(0, [1u8; 32], meta(0, 1, "daily"));
    let old_session = state.create_session(Some(0));
    let fresh_session = state.create_session(Some(0));
    {
        let mut sessions = state.sessions.lock();
        sessions.get_mut(&old_session).unwrap().last_activity =
            Instant::now() - Duration::from_secs(state.runtime_policy().idle_lock_secs + 1);
    }
    assert!(!state.idle_lock_due());

    {
        let mut sessions = state.sessions.lock();
        sessions.get_mut(&fresh_session).unwrap().last_activity =
            Instant::now() - Duration::from_secs(state.runtime_policy().idle_lock_secs + 1);
    }
    assert!(state.idle_lock_due());
}

#[test]
fn idle_lock_recheck_cancels_when_reauth_creates_fresh_session() {
    let dir = TempDir::new().unwrap();
    let state = AppState::new(dir.path().to_path_buf());

    state.unlock_compartment(0, [1u8; 32], meta(0, 1, "daily"));
    let stale_session = state.create_session(Some(0));
    state.revoke_session(&stale_session);

    assert!(state.idle_lock_due());
    assert!(state.begin_locking());

    let fresh_session = state.create_session(Some(0));
    assert!(!state.idle_lock_due_after_drain());
    state.finish_locking();
    assert!(state.verify_token(&fresh_session));
    assert!(state.is_unlocked());
}

#[test]
fn locking_state_rejects_session_validation_until_lock_finishes() {
    let dir = TempDir::new().unwrap();
    let state = AppState::new(dir.path().to_path_buf());

    state.unlock_compartment(0, [1u8; 32], meta(0, 1, "daily"));
    let session = state.create_session(Some(0));

    assert!(state.begin_locking());
    assert!(!state.verify_token(&session));
    state.lock_all();
    assert!(!state.is_locking());
}

#[test]
fn audit_log_roundtrip_and_limit() {
    let dir = TempDir::new().unwrap();
    let state = AppState::new(dir.path().to_path_buf());
    state.unlock_compartment(0, [1u8; 32], meta(0, 1, "daily"));

    state
        .record_audit_event(
            Some(0),
            AuditEventSpec::UnlockPassphrase {
                compartment_ids: vec![0],
                count: 1,
            },
        )
        .unwrap();
    state
        .record_audit_event(
            Some(0),
            AuditEventSpec::SecretSet {
                key: "db_pass".into(),
            },
        )
        .unwrap();
    state
        .record_audit_event(
            None,
            AuditEventSpec::SnapshotExport {
                file_count: 4,
                total_bytes: 128,
            },
        )
        .unwrap();

    let compartment_report = state.verify_audit_chain("compartment:0").unwrap();
    assert_eq!(compartment_report.status, "verified");
    assert_eq!(compartment_report.verified, 2);
    assert_eq!(compartment_report.broken, 0);

    let daemon_report = state.verify_audit_chain("daemon").unwrap();
    assert_eq!(daemon_report.status, "verified");
    assert_eq!(daemon_report.verified, 1);
    assert_eq!(daemon_report.broken, 0);

    let events = state
        .read_audit_events(AuditQuery {
            tail: 2,
            kind: None,
            since: None,
            key: None,
        })
        .unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].kind, "snapshot.export");
    assert_eq!(events[1].kind, "secret.set");
    assert_eq!(events[0].details["total_bytes"], serde_json::json!(128));
}

#[test]
fn compartment_audit_verify_requires_unlocked_key() {
    let dir = TempDir::new().unwrap();
    let state = AppState::new(dir.path().to_path_buf());

    let error = state.verify_audit_chain("compartment:0").unwrap_err();

    assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
}

#[test]
fn startup_recovery_summary_defaults_to_zero() {
    let dir = TempDir::new().unwrap();
    let state = AppState::new(dir.path().to_path_buf());

    assert_eq!(
        state.startup_recovery_summary(),
        StartupRecoverySummary::default()
    );
}
