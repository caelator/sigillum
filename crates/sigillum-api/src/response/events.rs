//! Server-sent event (SSE) payloads for `GET /api/events`.
//!
//! The daemon streams daemon-state transitions to subscribed clients so they
//! can stop polling (plan task 1.3, decision D-D). The vocabulary is
//! deliberately minimal — `snapshot`, `operation`, `queue`, and `status`
//! events — and versioned: every payload carries `v: 1`
//! ([`EVENTS_PROTOCOL_VERSION`]). Within 1.x the daemon may add new event
//! names and new optional fields, but never changes the meaning of existing
//! ones; clients MUST ignore events with unknown names and payloads with
//! unknown fields (this module's serde defaults make that the natural
//! behavior).
//!
//! Wire framing: each SSE frame carries the event name in the `event:` field
//! and one JSON payload in the `data:` field. The daemon also emits periodic
//! heartbeat comments (`:`-prefixed lines) which carry no payload and are
//! not represented here.

use serde::{Deserialize, Serialize};

use super::operations::Operation;

/// Protocol version stamped into every event payload's `v` field.
pub const EVENTS_PROTOCOL_VERSION: u32 = 1;

// ── Event names (the SSE `event:` field) ─────────────────────────

/// [`DaemonEvent::Snapshot`] — full resync state, sent once on connect and
/// again after a subscriber falls too far behind (see the daemon's lag
/// handling).
pub const EVENT_NAME_SNAPSHOT: &str = "snapshot";
/// [`DaemonEvent::Operation`] — an operation was created or transitioned
/// (state or progress).
pub const EVENT_NAME_OPERATION: &str = "operation";
/// [`DaemonEvent::Queue`] — a queue job changed state.
pub const EVENT_NAME_QUEUE: &str = "queue";
/// [`DaemonEvent::Status`] — lock, unlock, or compartment switch.
pub const EVENT_NAME_STATUS: &str = "status";

// ── Status event kinds ───────────────────────────────────────────

/// [`StatusEvent::kind`] — all compartments locked (manual lock, idle
/// auto-lock, or shutdown zeroization).
pub const STATUS_EVENT_LOCKED: &str = "locked";
/// [`StatusEvent::kind`] — a compartment was unlocked.
pub const STATUS_EVENT_UNLOCKED: &str = "unlocked";
/// [`StatusEvent::kind`] — a session switched its active compartment.
pub const STATUS_EVENT_COMPARTMENT_SWITCHED: &str = "compartment_switched";

/// `operation` event payload: the operation record after the transition.
///
/// The full [`Operation`] is included so subscribers never need a follow-up
/// `GET /api/operations/{id}`; create, state, and progress transitions all
/// share this one shape.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct OperationEvent {
    pub v: u32,
    pub operation: Operation,
}

/// `queue` event payload: a queue job's state transition.
///
/// Deliberately smaller than the full [`crate::QueueJob`] record: the job id
/// and new state are enough for a live view, and keeping payloads free of
/// payload/receipt fields avoids widening the information exposed on the
/// stream. `last_error` carries the human-readable transition reason when
/// one was recorded (blocked, retrying, failed, operator_action_required).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct QueueJobEvent {
    pub v: u32,
    pub job_id: String,
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

/// `status` event payload: daemon lock state or active-compartment changes.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct StatusEvent {
    pub v: u32,
    /// `locked`, `unlocked`, or `compartment_switched`
    /// ([`STATUS_EVENT_LOCKED`] and siblings).
    pub kind: String,
    /// The daemon-wide default active compartment after the transition, when
    /// any compartment is unlocked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_compartment_id: Option<usize>,
}

/// `snapshot` event payload: the first frame on every connection, and the
/// resync frame after a slow subscriber misses events.
///
/// Carries the lock status and the currently live (non-terminal) operations
/// so a fresh subscriber can render correct state without a second request.
/// Queue state is NOT included: the queue is durable and listable via
/// `GET /api/queue/jobs`, while the snapshot exists to cover state that only
/// exists in memory.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EventsSnapshot {
    pub v: u32,
    pub locked: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_compartment_id: Option<usize>,
    pub operations: Vec<Operation>,
}

/// A parsed daemon event: the SSE `event:` name plus its typed payload.
///
/// This is the client-side vocabulary of `GET /api/events`; the daemon
/// publishes the same values. Use [`DaemonEvent::event_name`] and
/// [`DaemonEvent::data_json`] to frame a value for the wire, and
/// [`DaemonEvent::from_sse`] to parse one back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DaemonEvent {
    Snapshot(EventsSnapshot),
    Operation(OperationEvent),
    Queue(QueueJobEvent),
    Status(StatusEvent),
}

impl DaemonEvent {
    /// The SSE `event:` field for this value.
    pub fn event_name(&self) -> &'static str {
        match self {
            DaemonEvent::Snapshot(_) => EVENT_NAME_SNAPSHOT,
            DaemonEvent::Operation(_) => EVENT_NAME_OPERATION,
            DaemonEvent::Queue(_) => EVENT_NAME_QUEUE,
            DaemonEvent::Status(_) => EVENT_NAME_STATUS,
        }
    }

    /// The JSON `data:` field for this value.
    pub fn data_json(&self) -> String {
        let result = match self {
            DaemonEvent::Snapshot(payload) => serde_json::to_string(payload),
            DaemonEvent::Operation(payload) => serde_json::to_string(payload),
            DaemonEvent::Queue(payload) => serde_json::to_string(payload),
            DaemonEvent::Status(payload) => serde_json::to_string(payload),
        };
        // Serialization of these plain-data structs is infallible in
        // practice; never let a payload kill the stream.
        result.unwrap_or_else(|_| "{}".to_string())
    }

    /// Parse one SSE frame. Returns `Ok(None)` for frames whose event name
    /// is not part of this vocabulary (heartbeats never reach here — they
    /// are SSE comments, not events), so older clients silently skip events
    /// introduced by newer daemons.
    pub fn from_sse(
        event_name: &str,
        data: &str,
    ) -> Result<Option<DaemonEvent>, serde_json::Error> {
        let parsed = match event_name {
            EVENT_NAME_SNAPSHOT => {
                DaemonEvent::Snapshot(serde_json::from_str::<EventsSnapshot>(data)?)
            }
            EVENT_NAME_OPERATION => {
                DaemonEvent::Operation(serde_json::from_str::<OperationEvent>(data)?)
            }
            EVENT_NAME_QUEUE => DaemonEvent::Queue(serde_json::from_str::<QueueJobEvent>(data)?),
            EVENT_NAME_STATUS => DaemonEvent::Status(serde_json::from_str::<StatusEvent>(data)?),
            _ => return Ok(None),
        };
        Ok(Some(parsed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{OPERATION_STATE_RUNNING, OperationProgress};

    fn sample_operation() -> Operation {
        Operation {
            id: "op-1".into(),
            kind: "inventory_scan_evm".into(),
            state: OPERATION_STATE_RUNNING.into(),
            progress: OperationProgress {
                processed: 3,
                total: None,
            },
            related_ids: vec!["job-1".into()],
            created_at_unix: 10,
            updated_at_unix: 12,
            completed_at_unix: None,
            error: None,
        }
    }

    fn roundtrip(event: DaemonEvent) {
        let json = event.data_json();
        let decoded = DaemonEvent::from_sse(event.event_name(), &json)
            .expect("payload parses")
            .expect("known event name");
        assert_eq!(decoded, event, "SSE roundtrip changed value: {json}");
    }

    #[test]
    fn all_event_kinds_roundtrip_through_sse_framing() {
        roundtrip(DaemonEvent::Snapshot(EventsSnapshot {
            v: EVENTS_PROTOCOL_VERSION,
            locked: false,
            active_compartment_id: Some(0),
            operations: vec![sample_operation()],
        }));
        roundtrip(DaemonEvent::Operation(OperationEvent {
            v: EVENTS_PROTOCOL_VERSION,
            operation: sample_operation(),
        }));
        roundtrip(DaemonEvent::Queue(QueueJobEvent {
            v: EVENTS_PROTOCOL_VERSION,
            job_id: "job-1".into(),
            state: "sent".into(),
            last_error: None,
        }));
        roundtrip(DaemonEvent::Queue(QueueJobEvent {
            v: EVENTS_PROTOCOL_VERSION,
            job_id: "job-2".into(),
            state: "blocked".into(),
            last_error: Some("waiting for gas".into()),
        }));
        roundtrip(DaemonEvent::Status(StatusEvent {
            v: EVENTS_PROTOCOL_VERSION,
            kind: STATUS_EVENT_COMPARTMENT_SWITCHED.into(),
            active_compartment_id: Some(1),
        }));
    }

    #[test]
    fn payloads_are_versioned_and_optional_fields_omitted() {
        let event = DaemonEvent::Status(StatusEvent {
            v: EVENTS_PROTOCOL_VERSION,
            kind: STATUS_EVENT_LOCKED.into(),
            active_compartment_id: None,
        });
        let json: serde_json::Value = serde_json::from_str(&event.data_json()).unwrap();
        assert_eq!(json["v"], 1);
        assert_eq!(json["kind"], "locked");
        assert!(json.get("active_compartment_id").is_none());

        let queue = DaemonEvent::Queue(QueueJobEvent {
            v: EVENTS_PROTOCOL_VERSION,
            job_id: "job-1".into(),
            state: "queued".into(),
            last_error: None,
        });
        let json: serde_json::Value = serde_json::from_str(&queue.data_json()).unwrap();
        assert_eq!(json["v"], 1);
        assert!(json.get("last_error").is_none());
    }

    #[test]
    fn unknown_event_names_are_skipped_not_errors() {
        assert_eq!(DaemonEvent::from_sse("telemetry", "{}").unwrap(), None);
    }

    #[test]
    fn malformed_payload_for_known_name_is_an_error() {
        assert!(DaemonEvent::from_sse(EVENT_NAME_STATUS, "not json").is_err());
    }
}
