//! Daemon-wide event bus feeding the `GET /api/events` SSE stream (plan
//! task 1.3, decision D-D).
//!
//! Publishers ([`AppState`](crate::AppState) wrappers around the operation
//! registry, queue state writes, and lock/compartment transitions) call
//! [`EventBus::publish`]; each connected SSE subscriber holds a
//! [`broadcast::Receiver`].
//!
//! Backpressure: the channel is bounded at [`EVENT_CHANNEL_CAPACITY`] per
//! subscriber. `publish` never blocks and never fails the caller — a
//! subscriber that falls more than the capacity behind loses the oldest
//! events and is handed a fresh `snapshot` (resync) by the SSE stream
//! instead, and a dropped subscriber simply stops receiving. Emitters are
//! therefore never stalled by slow or abandoned connections.

use sigillum_api::DaemonEvent;
use tokio::sync::broadcast;

/// Per-subscriber buffer. Chosen well above the burst size of the chatty
/// producers (a discovery scan emits one `operation` progress event per
/// address index; a drain one `queue` event per job transition); a
/// subscriber slower than this gets a resync snapshot rather than growing
/// memory.
pub(crate) const EVENT_CHANNEL_CAPACITY: usize = 256;

/// Fan-out hub for daemon events. Cheap to clone (shares one channel).
#[derive(Clone)]
pub(crate) struct EventBus {
    sender: broadcast::Sender<DaemonEvent>,
}

impl EventBus {
    pub(crate) fn new() -> Self {
        let (sender, _) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
        Self { sender }
    }

    /// Register a new subscriber. Receives only events published after this
    /// call (plus a resync snapshot if it lags), so SSE subscribers also get
    /// a synthesized snapshot frame on connect — see `routes::events`.
    pub(crate) fn subscribe(&self) -> broadcast::Receiver<DaemonEvent> {
        self.sender.subscribe()
    }

    /// A raw sender clone for publishers embedded in another component (the
    /// operation registry emits from inside its mutation methods).
    pub(crate) fn sender(&self) -> broadcast::Sender<DaemonEvent> {
        self.sender.clone()
    }

    /// Fan an event out to all current subscribers. Non-blocking: with no
    /// subscribers this is a no-op, and a full per-subscriber buffer drops
    /// that subscriber's oldest events (surfaced to it as `Lagged`, answered
    /// with a resync snapshot).
    pub(crate) fn publish(&self, event: DaemonEvent) {
        let _ = self.sender.send(event);
    }
}

#[cfg(test)]
mod tests {
    use sigillum_api::{EVENTS_PROTOCOL_VERSION, StatusEvent};

    use super::*;

    fn status_event() -> DaemonEvent {
        DaemonEvent::Status(StatusEvent {
            v: EVENTS_PROTOCOL_VERSION,
            kind: sigillum_api::STATUS_EVENT_LOCKED.into(),
            active_compartment_id: None,
        })
    }

    #[tokio::test]
    async fn slow_subscriber_lags_and_emitter_never_blocks() {
        let bus = EventBus::new();
        let mut receiver = bus.subscribe();

        // Publish well beyond capacity while the subscriber never reads:
        // every publish must return immediately.
        for _ in 0..(EVENT_CHANNEL_CAPACITY * 2) {
            bus.publish(status_event());
        }

        // The subscriber observes the lag, then drains only the retained
        // tail — the oldest events were dropped, not queued unboundedly.
        match receiver.recv().await {
            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                assert_eq!(skipped, EVENT_CHANNEL_CAPACITY as u64);
            }
            other => panic!("expected Lagged, got {other:?}"),
        }
        let mut drained = 0;
        while receiver.try_recv().is_ok() {
            drained += 1;
        }
        assert_eq!(drained, EVENT_CHANNEL_CAPACITY);
    }

    #[tokio::test]
    async fn dropped_subscriber_does_not_affect_remaining_subscribers() {
        let bus = EventBus::new();
        let dropped = bus.subscribe();
        let mut kept = bus.subscribe();
        drop(dropped);

        bus.publish(status_event());
        let received = tokio::time::timeout(std::time::Duration::from_secs(1), kept.recv())
            .await
            .expect("publish must reach remaining subscribers")
            .expect("channel stays open while the bus lives");
        assert!(matches!(received, DaemonEvent::Status(_)));
    }
}
