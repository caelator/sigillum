use std::io::{BufRead, BufReader, Write};
use std::path::Path;

use sigillum_api::AuditEvent as PublicAuditEvent;

use crate::json_store::{decode_json_document, encode_json_document_compact};

use super::StoredAuditEvent;

// Fixture for audit/migration.rs tests and legacy-format regression tests;
// live writes go through audit_db::append_event_chained.
pub(crate) fn append_audit_event(
    path: &Path,
    event: &StoredAuditEvent,
) -> Result<(), std::io::Error> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let line = encode_json_document_compact(event)?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(&line)?;
    file.write_all(b"\n")?;
    file.flush()?;
    Ok(())
}

// Regression-tests the legacy decode fallback that audit/migration.rs relies on.
pub(crate) fn read_recent_audit_events(
    path: &Path,
    limit: usize,
) -> Result<Vec<PublicAuditEvent>, std::io::Error> {
    let limit = limit.max(1);
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };

    let mut events = Vec::new();
    for line in BufReader::new(file).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let event: StoredAuditEvent = decode_json_document(path, line.as_bytes())?;
        events.push(event.to_public_event());
    }

    if events.len() > limit {
        events.drain(0..events.len() - limit);
    }
    events.reverse();
    Ok(events)
}
