use tempfile::TempDir;

use super::*;

#[test]
fn versioned_audit_event_roundtrips_to_public_shape() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("audit.log");
    append_audit_event(
        &path,
        &StoredAuditEvent {
            created_at_unix: 7,
            compartment_id: Some(0),
            spec: AuditEventSpec::QueueEnqueue {
                id: "job_1".into(),
                job_kind: AuditQueueJobKind::EthStealthNativeSweep,
            },
        },
    )
    .unwrap();

    let events = read_recent_audit_events(&path, 10).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, "queue.enqueue");
    assert_eq!(events[0].details["id"], json!("job_1"));
    assert_eq!(events[0].details["kind"], json!("eth_stealth_native_sweep"));
}

#[test]
fn audit_log_uses_single_line_versioned_documents() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("audit.log");
    append_audit_event(
        &path,
        &StoredAuditEvent {
            created_at_unix: 7,
            compartment_id: None,
            spec: AuditEventSpec::LockAll,
        },
    )
    .unwrap();

    let raw = std::fs::read_to_string(&path).unwrap();
    let lines: Vec<&str> = raw.lines().collect();
    assert_eq!(lines.len(), 1);
    let saved: Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(saved["schema"], json!("sigillum.audit-event"));
    assert_eq!(saved["schema_version"], json!(1));
    assert_eq!(saved["data"]["kind"], json!("lock.all"));
}

#[test]
fn legacy_public_audit_events_still_load() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("audit.log");
    std::fs::write(
        &path,
        format!(
            "{}\n",
            serde_json::to_string(&PublicAuditEvent {
                created_at_unix: 7,
                kind: "fido2.setup".into(),
                compartment_id: Some(0),
                details: json!({
                    "label": "primary",
                    "compartments": 3,
                    "total_keys": 1,
                }),
            })
            .unwrap()
        ),
    )
    .unwrap();

    let events = read_recent_audit_events(&path, 10).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, "fido2.setup");
    assert_eq!(events[0].details["compartment_count"], json!(3));
}

#[test]
fn unsupported_legacy_event_kind_returns_error() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("audit.log");
    std::fs::write(
        &path,
        format!(
            "{}\n",
            serde_json::to_string(&PublicAuditEvent {
                created_at_unix: 7,
                kind: "unknown.kind".into(),
                compartment_id: None,
                details: json!({}),
            })
            .unwrap()
        ),
    )
    .unwrap();

    let error = read_recent_audit_events(&path, 10).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    assert!(error.to_string().contains("unsupported audit event kind"));
}
