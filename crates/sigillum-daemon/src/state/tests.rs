use super::*;

const POISON_CHILD_ENV: &str = "SIGILLUM_TEST_POISONED_MUTEX_CHILD";

#[test]
fn resilient_mutex_normal_lock_is_unchanged() {
    let mutex = ResilientMutex::new(vec![1_u8]);
    mutex.lock().push(2);
    assert_eq!(&*mutex.lock(), &[1, 2]);
}

#[test]
fn resilient_mutex_poison_child() {
    let Ok(mode) = std::env::var(POISON_CHILD_ENV) else {
        return;
    };
    let mutex = std::sync::Arc::new(ResilientMutex::new(vec!["secret-state"]));
    let poisoner = mutex.clone();
    let _ = std::thread::spawn(move || {
        let _guard = poisoner.lock();
        panic!("intentional poison for subprocess test");
    })
    .join();

    match mode.as_str() {
        "lock" => drop(mutex.lock()),
        "debug" => drop(format!("{mutex:?}")),
        other => panic!("unknown poison child mode: {other}"),
    }
}

#[test]
fn resilient_mutex_poison_aborts_lock_and_debug_paths() {
    let current_test_binary = std::env::current_exe().expect("current test binary");
    for mode in ["lock", "debug"] {
        let status = std::process::Command::new(&current_test_binary)
            .args([
                "--exact",
                "state::tests::resilient_mutex_poison_child",
                "--nocapture",
            ])
            .env(POISON_CHILD_ENV, mode)
            .status()
            .expect("poison child should launch");
        assert!(!status.success(), "poisoned {mode} path must abort");
    }
}
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
    let state = AppState::new(dir.path().to_path_buf()).expect("app state should initialize");

    state.unlock_compartment(0, [1u8; 32], meta(0, 1, "daily"));
    state.unlock_compartment(1, [2u8; 32], meta(1, 2, "secure"));

    let session_a = state.create_session(Some(0));
    let session_b = state.create_session(Some(0));

    state.switch_active_for(&session_a, 1).unwrap();

    assert_eq!(state.active_compartment_id_for(&session_a), Some(1));
    assert_eq!(state.active_compartment_id_for(&session_b), Some(0));
}

#[test]
fn rotating_session_active_invalidates_old_token_and_preserves_metadata() {
    let dir = TempDir::new().unwrap();
    let state = AppState::new(dir.path().to_path_buf()).expect("app state should initialize");
    state.unlock_compartment(0, [1u8; 32], meta(0, 1, "daily"));
    state.unlock_compartment(1, [2u8; 32], meta(1, 2, "secure"));

    let (session, _) = state.create_capability_session(
        Some(0),
        vec!["deposits:read".into()],
        Duration::from_secs(90),
    );
    let before = state.sessions.lock().active.get(&session).unwrap().clone();
    let replacement = state.rotate_session_active_for(&session, 1).unwrap();
    let after = state
        .sessions
        .lock()
        .active
        .get(&replacement)
        .unwrap()
        .clone();

    assert_ne!(replacement, session);
    assert!(!state.verify_token(&session));
    assert_eq!(state.active_compartment_id_for(&replacement), Some(1));
    assert!(state.session_has_scope(&replacement, "deposits:read"));
    assert!(!state.session_is_full(&replacement));
    assert!(
        !state.verify_full_or_retired_lock_token(&replacement),
        "an active capability session must not authorize process-global Lock"
    );
    assert!(
        !state.verify_full_or_retired_lock_token(&session),
        "rotating a capability session must not grant its predecessor Lock authority"
    );
    assert_eq!(after.created_at, before.created_at);
    assert_eq!(after.expires_at, before.expires_at);
    assert_eq!(after.last_activity, before.last_activity);
    assert_eq!(after.scopes, before.scopes);
}

#[test]
fn absent_or_revoked_session_never_falls_back_to_default_compartment() {
    let dir = TempDir::new().unwrap();
    let state = AppState::new(dir.path().to_path_buf()).expect("app state should initialize");
    state.unlock_compartment(0, [1u8; 32], meta(0, 1, "daily"));

    assert_eq!(state.active_compartment_id_for("missing"), None);

    let session = state.create_session(None);
    assert_eq!(state.active_compartment_id_for(&session), Some(0));
    state.revoke_session(&session);
    assert_eq!(state.active_compartment_id_for(&session), None);
}

#[test]
fn full_session_rotation_retains_only_immediate_lock_predecessor() {
    let dir = TempDir::new().unwrap();
    let state = AppState::new(dir.path().to_path_buf()).expect("app state should initialize");
    state.unlock_compartment(0, [1u8; 32], meta(0, 1, "daily"));
    state.unlock_compartment(1, [2u8; 32], meta(1, 2, "secure"));

    let first = state.create_session(Some(0));
    let second = state.rotate_session_active_for(&first, 1).unwrap();

    assert!(!state.verify_token(&first));
    assert!(state.verify_full_or_retired_lock_token(&first));
    assert!(state.verify_token(&second));

    let third = state.rotate_session_active_for(&second, 0).unwrap();
    assert!(
        !state.verify_full_or_retired_lock_token(&first),
        "a second rotation must retire the older predecessor"
    );
    assert!(state.verify_full_or_retired_lock_token(&second));
    assert!(state.verify_token(&third));
    assert_eq!(state.sessions.lock().retired_len(), 1);
}

#[test]
fn revoking_rotated_successor_also_revokes_lock_predecessor() {
    let dir = TempDir::new().unwrap();
    let state = AppState::new(dir.path().to_path_buf()).expect("app state should initialize");
    state.unlock_compartment(0, [1u8; 32], meta(0, 1, "daily"));
    state.unlock_compartment(1, [2u8; 32], meta(1, 2, "secure"));

    let predecessor = state.create_session(Some(0));
    let successor = state.rotate_session_active_for(&predecessor, 1).unwrap();
    assert!(state.verify_full_or_retired_lock_token(&predecessor));

    state.revoke_session(&successor);

    assert!(!state.verify_full_or_retired_lock_token(&predecessor));
    assert_eq!(state.sessions.lock().retired_len(), 0);
}

#[test]
fn idle_rotated_successor_removes_its_lock_predecessor() {
    let dir = TempDir::new().unwrap();
    let state = AppState::new(dir.path().to_path_buf()).expect("app state should initialize");
    state.unlock_compartment(0, [1u8; 32], meta(0, 1, "daily"));
    state.unlock_compartment(1, [2u8; 32], meta(1, 2, "secure"));

    let predecessor = state.create_session(Some(0));
    let successor = state.rotate_session_active_for(&predecessor, 1).unwrap();
    state
        .sessions
        .lock()
        .active
        .get_mut(&successor)
        .unwrap()
        .last_activity =
        Instant::now() - Duration::from_secs(state.runtime_policy().idle_lock_secs + 1);

    assert!(!state.verify_full_or_retired_lock_token(&predecessor));
    assert_eq!(state.session_count(), 0);
    assert_eq!(state.sessions.lock().retired_len(), 0);
}

#[test]
fn expired_rotated_successor_removes_its_lock_predecessor() {
    let dir = TempDir::new().unwrap();
    let state = AppState::new(dir.path().to_path_buf()).expect("app state should initialize");
    state.unlock_compartment(0, [1u8; 32], meta(0, 1, "daily"));
    state.unlock_compartment(1, [2u8; 32], meta(1, 2, "secure"));

    let predecessor = state.create_session(Some(0));
    let successor = state.rotate_session_active_for(&predecessor, 1).unwrap();
    state
        .sessions
        .lock()
        .active
        .get_mut(&successor)
        .unwrap()
        .expires_at = Instant::now() - Duration::from_secs(1);

    assert!(!state.verify_full_or_retired_lock_token(&predecessor));
    assert_eq!(state.session_count(), 0);
    assert_eq!(state.sessions.lock().retired_len(), 0);
}

#[test]
fn capacity_eviction_removes_rotated_successor_lock_predecessor() {
    let dir = TempDir::new().unwrap();
    let state = AppState::new(dir.path().to_path_buf()).expect("app state should initialize");
    state.unlock_compartment(0, [1u8; 32], meta(0, 1, "daily"));
    state.unlock_compartment(1, [2u8; 32], meta(1, 2, "secure"));

    let predecessor = state.create_session(Some(0));
    let _successor = state.rotate_session_active_for(&predecessor, 1).unwrap();
    for _ in 0..session_registry::MAX_SESSIONS {
        state.create_session(Some(0));
    }

    assert_eq!(state.session_count(), session_registry::MAX_SESSIONS);
    assert!(!state.verify_full_or_retired_lock_token(&predecessor));
    assert_eq!(state.sessions.lock().retired_len(), 0);
}

#[test]
fn invalidating_all_sessions_clears_rotated_lock_predecessors() {
    let dir = TempDir::new().unwrap();
    let state = AppState::new(dir.path().to_path_buf()).expect("app state should initialize");
    state.unlock_compartment(0, [1u8; 32], meta(0, 1, "daily"));
    state.unlock_compartment(1, [2u8; 32], meta(1, 2, "secure"));

    let predecessor = state.create_session(Some(0));
    let _successor = state.rotate_session_active_for(&predecessor, 1).unwrap();
    state.invalidate_all_sessions();

    assert!(!state.verify_full_or_retired_lock_token(&predecessor));
    assert_eq!(state.session_count(), 0);
    assert_eq!(state.sessions.lock().retired_len(), 0);
}

#[test]
fn removing_active_compartment_repoints_sessions() {
    let dir = TempDir::new().unwrap();
    let state = AppState::new(dir.path().to_path_buf()).expect("app state should initialize");

    state.unlock_compartment(0, [1u8; 32], meta(0, 1, "daily"));
    state.unlock_compartment(1, [2u8; 32], meta(1, 2, "secure"));

    let session = state.create_session(Some(1));
    state.remove_compartment(1);

    assert_eq!(state.active_compartment_id_for(&session), Some(0));
}

#[test]
fn lock_all_clears_sessions_and_vault_instances() {
    let dir = TempDir::new().unwrap();
    let state = AppState::new(dir.path().to_path_buf()).expect("app state should initialize");

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
    let state = AppState::new(dir.path().to_path_buf()).expect("app state should initialize");

    state.unlock_compartment(0, [1u8; 32], meta(0, 1, "daily"));
    let session = state.create_session(Some(0));
    {
        let mut sessions = state.sessions.lock();
        let session = sessions.active.get_mut(&session).unwrap();
        session.last_activity =
            Instant::now() - Duration::from_secs(state.runtime_policy().idle_lock_secs + 1);
    }

    assert!(!state.verify_token(&session));
    assert_eq!(state.session_count(), 0);
}

#[test]
fn idle_lock_due_requires_all_sessions_to_be_idle() {
    let dir = TempDir::new().unwrap();
    let state = AppState::new(dir.path().to_path_buf()).expect("app state should initialize");

    state.unlock_compartment(0, [1u8; 32], meta(0, 1, "daily"));
    let old_session = state.create_session(Some(0));
    let fresh_session = state.create_session(Some(0));
    {
        let mut sessions = state.sessions.lock();
        sessions.active.get_mut(&old_session).unwrap().last_activity =
            Instant::now() - Duration::from_secs(state.runtime_policy().idle_lock_secs + 1);
    }
    assert!(!state.idle_lock_due());

    {
        let mut sessions = state.sessions.lock();
        sessions
            .active
            .get_mut(&fresh_session)
            .unwrap()
            .last_activity =
            Instant::now() - Duration::from_secs(state.runtime_policy().idle_lock_secs + 1);
    }
    assert!(state.idle_lock_due());
}

#[test]
fn idle_lock_recheck_cancels_when_reauth_creates_fresh_session() {
    let dir = TempDir::new().unwrap();
    let state = AppState::new(dir.path().to_path_buf()).expect("app state should initialize");

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
    let state = AppState::new(dir.path().to_path_buf()).expect("app state should initialize");

    state.unlock_compartment(0, [1u8; 32], meta(0, 1, "daily"));
    let session = state.create_session(Some(0));

    assert!(state.begin_locking());
    assert!(!state.verify_token(&session));
    state.lock_all();
    assert!(!state.is_locking());
}

#[test]
fn broadcast_admission_linearizes_with_lock_latch() {
    let dir = TempDir::new().unwrap();
    let state = AppState::new(dir.path().to_path_buf()).expect("app state should initialize");

    state.unlock_compartment(0, [1u8; 32], meta(0, 1, "daily"));
    assert!(state.admit_broadcast_if_ready());

    assert!(state.begin_locking());
    assert!(state.is_locking());
    assert!(
        !state.admit_broadcast_if_ready(),
        "an admission ordered after the Lock latch must fail"
    );

    state.lock_all();
    assert!(!state.is_locking());
}

#[test]
fn audit_log_roundtrip_and_limit() {
    let dir = TempDir::new().unwrap();
    let state = AppState::new(dir.path().to_path_buf()).expect("app state should initialize");
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
    let state = AppState::new(dir.path().to_path_buf()).expect("app state should initialize");

    let error = state.verify_audit_chain("compartment:0").unwrap_err();

    assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
}

#[test]
fn startup_recovery_summary_defaults_to_zero() {
    let dir = TempDir::new().unwrap();
    let state = AppState::new(dir.path().to_path_buf()).expect("app state should initialize");

    assert_eq!(
        state.startup_recovery_summary(),
        StartupRecoverySummary::default()
    );
}
