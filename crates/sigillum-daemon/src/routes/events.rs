//! `GET /api/events` — the daemon's server-sent events (SSE) channel (plan
//! task 1.3, decision D-D).
//!
//! Clients subscribe once and receive daemon-state transitions instead of
//! polling: `operation` (registry create/state/progress), `queue` (job state
//! transitions), and `status` (compartment/lifecycle state) events, plus a
//! `snapshot` frame on connect so a subscriber syncs without a second
//! request. Lock invalidates the session, emits one final non-sensitive
//! `locked` status frame, and closes the stream; revocation or expiry closes
//! it without another frame. Payloads are the versioned DTOs from
//! `sigillum_api::response::events` (`v: 1`); heartbeat comments keep
//! intermediaries from killing the stream.
//!
//! ## Authentication and the idle-lock rule
//!
//! The stream uses the same bearer session model as the rest of the API,
//! with one deliberate extension: because browser `EventSource` cannot set
//! headers, the token is also accepted as a `?session=` query parameter when
//! the daemon is bound to loopback. Non-loopback server entry points disable
//! that fallback while retaining `Authorization` header authentication.
//! URLs are inherently leak-prone (logs, browser history, proxies), so
//! non-browser clients should always prefer the header.
//!
//! The verify is PASSIVE (`AppState::verify_token_passive` via
//! [`crate::service::require_passive_full_session_token`]): connecting, and
//! staying connected, does NOT refresh the session's idle-activity clock, so
//! an always-open events tab cannot defeat the vault auto-lock. The stream
//! revalidates passively before every frame and on a short timer, terminating
//! promptly after session revocation, expiry, or the terminal lock frame.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::Extension;
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use futures_core::Stream;
use serde::Deserialize;
use tokio::sync::broadcast;

use sigillum_api::DaemonEvent;

use crate::{AppState, EventQuerySessionPolicy};

use super::{bearer_token, sec_headers, service_response};

/// Heartbeat cadence. Comfortable margin below the 30–60 s idle timeouts
/// typical of local proxies and browser stacks; the daemon binds loopback,
/// but desktop shells and dev tunnels still sit in between.
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(25);

/// Bound how long a revoked or lock-cleared session can remain connected
/// while the event bus is idle. Verification is passive, so these checks do
/// not extend the session's idle lifetime.
const SESSION_REVALIDATE_INTERVAL: Duration = Duration::from_secs(1);

/// Query-carried session token for `EventSource` clients. See the module
/// docs for the loopback-only trade-off.
#[derive(Debug, Deserialize)]
pub(crate) struct EventsQuery {
    session: Option<String>,
}

pub(crate) async fn get_events(
    State(state): State<Arc<AppState>>,
    Extension(query_policy): Extension<EventQuerySessionPolicy>,
    headers: HeaderMap,
    Query(query): Query<EventsQuery>,
) -> Response {
    let candidate = bearer_token(&headers).or_else(|| {
        query_policy
            .allow_query_token
            .then_some(query.session)
            .flatten()
    });
    let token =
        match crate::service::require_passive_full_session_token(&state, candidate.as_deref()) {
            Ok(token) => token.to_owned(),
            Err(error) => return service_response::<()>(Err(error)),
        };

    // Subscribe BEFORE building the snapshot so no transition can fall
    // between the two; the subscriber may therefore receive an event the
    // snapshot already reflects (events carry full post-transition records,
    // so re-applying is idempotent).
    let receiver = state.subscribe_events();
    sec_headers(
        Sse::new(events_stream(state, receiver, token))
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
    token: String,
) -> impl Stream<Item = Result<Event, Infallible>> + Send {
    let mut revalidate = tokio::time::interval(SESSION_REVALIDATE_INTERVAL);
    revalidate.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    futures_util::stream::unfold(
        (state, receiver, token, true, false, revalidate),
        |(state, mut receiver, token, pending_snapshot, terminate_after_frame, mut revalidate)| async move {
            if terminate_after_frame {
                return None;
            }
            if crate::service::require_passive_full_session_token(&state, Some(&token)).is_err() {
                let terminal = terminal_locked_frame(&state, &mut receiver)?;
                return Some((
                    Ok(terminal),
                    (state, receiver, token, false, true, revalidate),
                ));
            }
            if pending_snapshot {
                let snapshot = DaemonEvent::Snapshot(state.events_snapshot());
                return Some((
                    Ok(to_sse_frame(&snapshot)),
                    (state, receiver, token, false, false, revalidate),
                ));
            }

            loop {
                tokio::select! {
                    _ = revalidate.tick() => {
                        if crate::service::require_passive_full_session_token(
                            &state,
                            Some(&token),
                        )
                        .is_err()
                        {
                            let terminal = terminal_locked_frame(&state, &mut receiver)?;
                            return Some((
                                Ok(terminal),
                                (state, receiver, token, false, true, revalidate),
                            ));
                        }
                    }
                    received = receiver.recv() => {
                        // Revalidate immediately before every live event or
                        // resync snapshot. This closes the authorization gap
                        // between stream creation and later bus activity.
                        if crate::service::require_passive_full_session_token(
                            &state,
                            Some(&token),
                        )
                        .is_err()
                        {
                            let terminal = terminal_locked_frame(&state, &mut receiver)?;
                            return Some((
                                Ok(terminal),
                                (state, receiver, token, false, true, revalidate),
                            ));
                        }
                        let next = match received {
                            Ok(event) => to_sse_frame(&event),
                            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                                // The subscriber fell behind and events were dropped:
                                // resync it with a full snapshot instead of the missed
                                // transitions.
                                tracing::debug!(
                                    skipped,
                                    "events subscriber lagged; sending resync snapshot"
                                );
                                to_sse_frame(&DaemonEvent::Snapshot(state.events_snapshot()))
                            }
                            // The bus lives as long as the daemon, so Closed is
                            // effectively unreachable; end the stream gracefully if it
                            // ever happens.
                            Err(broadcast::error::RecvError::Closed) => return None,
                        };
                        return Some((
                            Ok(next),
                            (state, receiver, token, false, false, revalidate),
                        ));
                    }
                }
            }
        },
    )
}

/// Preserve the stable lifecycle contract without extending authorization:
/// once the vault is locked, a now-invalid subscriber receives exactly one
/// non-sensitive terminal `locked` status frame and then EOF. Other invalid
/// sessions (revoked or expired while the vault remains unlocked) end without
/// another frame.
fn terminal_locked_frame(
    state: &AppState,
    receiver: &mut broadcast::Receiver<DaemonEvent>,
) -> Option<Event> {
    if !state.is_unlocked() {
        return Some(locked_status_frame());
    }

    // A fast re-unlock can make the current state unlocked before this
    // invalidated stream is polled. Drain only already-buffered events and
    // preserve a preceding terminal lock notification; every other event is
    // discarded so stale authorization can never observe post-lock data.
    loop {
        match receiver.try_recv() {
            Ok(DaemonEvent::Status(status)) if status.kind == sigillum_api::STATUS_EVENT_LOCKED => {
                return Some(to_sse_frame(&DaemonEvent::Status(status)));
            }
            Ok(_) | Err(broadcast::error::TryRecvError::Lagged(_)) => continue,
            Err(broadcast::error::TryRecvError::Empty | broadcast::error::TryRecvError::Closed) => {
                return None;
            }
        }
    }
}

fn locked_status_frame() -> Event {
    to_sse_frame(&DaemonEvent::Status(sigillum_api::StatusEvent {
        v: sigillum_api::EVENTS_PROTOCOL_VERSION,
        kind: sigillum_api::STATUS_EVENT_LOCKED.into(),
        active_compartment_id: None,
    }))
}

/// Frame one daemon event as an SSE frame: `event:` name + JSON `data:`.
fn to_sse_frame(event: &DaemonEvent) -> Event {
    Event::default()
        .event(event.event_name())
        .data(event.data_json())
}

#[cfg(test)]
mod tests {
    use sigillum_fido2::config::CompartmentMeta;
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
        let meta = CompartmentMeta {
            id: 0,
            label: "default".into(),
            threshold: 1,
            passphrase_mode: None,
        };
        state.unlock_compartment(0, [1_u8; 32], meta.clone());
        let token = state.create_session(Some(0));
        let receiver = state.subscribe_events();
        let mut stream = Box::pin(events_stream(state.clone(), receiver, token));

        // Frame 1: connect-time snapshot (unlocked, no operations).
        let first = format!("{:?}", next_frame(&mut stream).await);
        assert!(first.contains("event: snapshot"), "first frame: {first}");

        // Overflow the subscriber's channel from the same bus.
        for _ in 0..(EVENT_CHANNEL_CAPACITY * 2) {
            state.unlock_compartment(0, [1_u8; 32], meta.clone());
        }

        // Frame 2: a resync snapshot replaces the dropped events.
        let second = format!("{:?}", next_frame(&mut stream).await);
        assert!(second.contains("event: snapshot"), "resync frame: {second}");
    }
}
