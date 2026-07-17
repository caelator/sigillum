//! `GET /api/events` — the daemon's server-sent events (SSE) channel (plan
//! task 1.3, decision D-D).
//!
//! Clients subscribe once and receive daemon-state transitions instead of
//! polling: `operation` (registry create/state/progress), `queue` (job state
//! transitions), and `status` (lock, unlock, compartment switch) events, plus
//! a `snapshot` frame on connect so a subscriber syncs without a second
//! request. Payloads are the versioned DTOs from
//! `sigillum_api::response::events` (`v: 1`); heartbeat comments keep
//! intermediaries from killing the stream.
//!
//! ## Authentication and the idle-lock rule
//!
//! The stream uses the same bearer session model as the rest of the API,
//! with one deliberate extension: because browser `EventSource` cannot set
//! headers, the token is also accepted as a `?session=` query parameter.
//! That trade-off is loopback-only by design — the daemon binds to
//! localhost and CORS stays pinned to the loopback origin, so the token in
//! a URL never leaves the machine; non-browser clients should prefer the
//! `Authorization` header (URLs are inherently leak-prone: logs, browser
//! history, proxies).
//!
//! The verify is PASSIVE (`AppState::verify_token_passive` via
//! [`crate::service::require_passive_full_session_token`]): connecting, and
//! staying connected, does NOT refresh the session's idle-activity clock, so
//! an always-open events tab cannot defeat the vault auto-lock. The stream
//! itself performs no further verifies — after the idle timeout the session
//! is evicted exactly as if the stream were not there.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use futures_core::Stream;
use serde::Deserialize;
use tokio::sync::broadcast;

use sigillum_api::DaemonEvent;

use crate::AppState;

use super::{bearer_token, sec_headers, service_response};

/// Heartbeat cadence. Comfortable margin below the 30–60 s idle timeouts
/// typical of local proxies and browser stacks; the daemon binds loopback,
/// but desktop shells and dev tunnels still sit in between.
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(25);

/// Query-carried session token for `EventSource` clients. See the module
/// docs for the loopback-only trade-off.
#[derive(Debug, Deserialize)]
pub(crate) struct EventsQuery {
    session: Option<String>,
}

pub(crate) async fn get_events(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<EventsQuery>,
) -> Response {
    let token = bearer_token(&headers).or(query.session);
    if let Err(error) = crate::service::require_passive_full_session_token(&state, token.as_deref())
    {
        return service_response::<()>(Err(error));
    }

    // Subscribe BEFORE building the snapshot so no transition can fall
    // between the two; the subscriber may therefore receive an event the
    // snapshot already reflects (events carry full post-transition records,
    // so re-applying is idempotent).
    let receiver = state.subscribe_events();
    sec_headers(
        Sse::new(events_stream(state, receiver))
            .keep_alive(KeepAlive::new().interval(HEARTBEAT_INTERVAL).text("hb"))
            .into_response(),
    )
}

/// The stream behind `GET /api/events`: yields the connect-time snapshot,
/// then live bus events. A subscriber that falls more than the channel
/// capacity behind receives a FRESH snapshot (resync) in place of the
/// dropped events rather than an error or a stalled stream.
fn events_stream(
    state: Arc<AppState>,
    receiver: broadcast::Receiver<DaemonEvent>,
) -> impl Stream<Item = Result<Event, Infallible>> + Send {
    futures_util::stream::unfold(
        (state, receiver, true),
        |(state, mut receiver, pending_snapshot)| async move {
            if pending_snapshot {
                let snapshot = DaemonEvent::Snapshot(state.events_snapshot());
                return Some((Ok(to_sse_frame(&snapshot)), (state, receiver, false)));
            }
            let next = match receiver.recv().await {
                Ok(event) => to_sse_frame(&event),
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    // The subscriber fell behind and events were dropped:
                    // resync it with a full snapshot instead of the missed
                    // transitions.
                    tracing::debug!(skipped, "events subscriber lagged; sending resync snapshot");
                    to_sse_frame(&DaemonEvent::Snapshot(state.events_snapshot()))
                }
                // The bus lives as long as the daemon, so Closed is
                // effectively unreachable; end the stream gracefully if it
                // ever happens.
                Err(broadcast::error::RecvError::Closed) => return None,
            };
            Some((Ok(next), (state, receiver, false)))
        },
    )
}

/// Frame one daemon event as an SSE frame: `event:` name + JSON `data:`.
fn to_sse_frame(event: &DaemonEvent) -> Event {
    Event::default()
        .event(event.event_name())
        .data(event.data_json())
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use crate::events::EVENT_CHANNEL_CAPACITY;

    /// Poll one item off an infallible SSE stream.
    async fn next_frame<S>(stream: &mut std::pin::Pin<Box<S>>) -> Event
    where
        S: Stream<Item = Result<Event, Infallible>>,
    {
        std::future::poll_fn(|cx| stream.as_mut().poll_next(cx))
            .await
            .transpose()
            .unwrap()
            .expect("stream yields a frame")
    }

    /// The first frame on a connection is always the snapshot, and a
    /// subscriber that falls more than the channel capacity behind gets a
    /// fresh resync snapshot instead of the dropped events.
    #[tokio::test]
    async fn stream_yields_snapshot_then_resyncs_on_lag() {
        let dir = TempDir::new().unwrap();
        let state = Arc::new(AppState::new(dir.path().to_path_buf()).unwrap());
        let receiver = state.subscribe_events();
        let mut stream = Box::pin(events_stream(state.clone(), receiver));

        // Frame 1: connect-time snapshot (locked, no operations).
        let first = format!("{:?}", next_frame(&mut stream).await);
        assert!(first.contains("event: snapshot"), "first frame: {first}");

        // Overflow the subscriber's channel from the same bus.
        for _ in 0..(EVENT_CHANNEL_CAPACITY * 2) {
            state.lock_all(); // publishes one status event each call
        }

        // Frame 2: a resync snapshot replaces the dropped events.
        let second = format!("{:?}", next_frame(&mut stream).await);
        assert!(second.contains("event: snapshot"), "resync frame: {second}");
    }
}
