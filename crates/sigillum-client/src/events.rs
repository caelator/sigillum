//! Subscription to the daemon's `GET /api/events` SSE channel (plan task
//! 1.3, decision D-D).
//!
//! [`SigillumClient::subscribe_events`] opens the stream and returns an
//! [`EventSubscription`] — a [`Stream`] of parsed, typed
//! [`DaemonEvent`]s (an inherent `next()` is provided for callers that
//! prefer an async pull loop over importing the `Stream` machinery).
//!
//! ## Reconnecting
//!
//! The stream ends (`next()` yields `None`) when the daemon closes the
//! connection — process shutdown, a network hiccup on the loopback path — or
//! yields an error when the byte stream fails or a known event arrives
//! malformed. There is deliberately no built-in retry: callers that want a
//! permanent feed should re-call [`SigillumClient::subscribe_events`] on
//! termination, after re-authenticating if the session expired in the
//! meantime. Every new connection begins with a `snapshot` event, so resync
//! after a reconnect is automatic — apply the snapshot, then keep applying
//! events.
//!
//! Note that the subscription is a PASSIVE read on the daemon: it
//! authenticates the session but does not refresh its idle-activity clock,
//! so an always-open subscription cannot defeat the vault auto-lock. A
//! session that goes idle is evicted and the daemon closes its stream after
//! passive revalidation; `next()` then yields `None` and the client must
//! unlock again before reconnecting.

use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::Bytes;
use futures_core::Stream;
use reqwest::{Method, StatusCode};
use sigillum_api::DaemonEvent;

use crate::{ClientError, SigillumClient};

impl SigillumClient {
    /// Open a subscription to the daemon's event stream.
    ///
    /// Uses the session token exactly like every other call (a daemon served
    /// on a verified loopback listener also accepts a `?session=` query token
    /// for browser `EventSource`, which this client never needs). See the
    /// module docs for the reconnect contract and passive-read idle-lock rule.
    pub async fn subscribe_events(&self) -> Result<EventSubscription, ClientError> {
        let builder = self.stream_request(Method::GET, "/api/events");
        let response = builder.send().await?;
        let status = response.status();
        if status == StatusCode::UNAUTHORIZED {
            self.clear_session_token();
        }
        if !status.is_success() {
            let text = response.text().await?;
            let (message, code, fields) = match serde_json::from_str::<crate::ErrorResponse>(&text)
            {
                Ok(error) => {
                    let code =
                        (error.code != sigillum_api::error_codes::UNKNOWN).then_some(error.code);
                    (error.error, code, error.fields.unwrap_or_default())
                }
                Err(_) => {
                    let message = if text.is_empty() {
                        format!("request failed with status {status}")
                    } else {
                        text
                    };
                    (message, None, Vec::new())
                }
            };
            return Err(ClientError::Api {
                status,
                message,
                code,
                fields,
            });
        }
        Ok(EventSubscription::new(response.bytes_stream()))
    }
}

/// A live SSE subscription: a stream of parsed [`DaemonEvent`]s.
///
/// Heartbeat comments and events with names outside the v1 vocabulary are
/// skipped transparently, so a newer daemon's additional event kinds never
/// break an older client.
pub struct EventSubscription {
    bytes: Pin<Box<dyn Stream<Item = Result<Bytes, reqwest::Error>> + Send>>,
    /// Undecoded bytes not yet terminated by a blank line.
    buffer: Vec<u8>,
    /// `event:` field of the frame currently being accumulated.
    frame_event: Option<String>,
    /// `data:` lines of the frame currently being accumulated.
    frame_data: Vec<String>,
    /// The byte stream has ended; flush any trailing partial frame.
    byte_stream_done: bool,
}

impl EventSubscription {
    fn new(
        byte_stream: impl Stream<Item = Result<Bytes, reqwest::Error>> + Send + 'static,
    ) -> Self {
        Self {
            bytes: Box::pin(byte_stream),
            buffer: Vec::new(),
            frame_event: None,
            frame_data: Vec::new(),
            byte_stream_done: false,
        }
    }

    /// Pull the next parsed event. Equivalent to (and implemented via) the
    /// [`Stream`] impl; `None` means the daemon ended the stream — see the
    /// module docs for the reconnect contract.
    pub async fn next(&mut self) -> Option<Result<DaemonEvent, ClientError>> {
        std::future::poll_fn(|cx| Pin::new(&mut *self).poll_next(cx)).await
    }

    /// Decode complete frames from the buffer. Returns the next parsed
    /// event when one is ready, skipping heartbeats and unknown event names.
    fn take_parsed_event(&mut self) -> Option<Result<DaemonEvent, ClientError>> {
        loop {
            let frame = match self.buffer.windows(2).position(|w| w == b"\n\n") {
                Some(end) => self.buffer.drain(..end + 2).collect::<Vec<u8>>(),
                None if self.byte_stream_done && !self.buffer.is_empty() => {
                    // Connection closed mid-frame: flush the remainder as one
                    // final frame (SSE allows EOF to terminate a frame).
                    std::mem::take(&mut self.buffer)
                }
                None => return None,
            };
            for line in String::from_utf8_lossy(&frame).split('\n') {
                let line = line.strip_suffix('\r').unwrap_or(line);
                if line.is_empty() {
                    continue;
                }
                if line.starts_with(':') {
                    // Heartbeat / comment: no field content.
                    continue;
                }
                let (field, value) = match line.split_once(':') {
                    Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
                    None => (line, ""),
                };
                match field {
                    "event" => self.frame_event = Some(value.to_string()),
                    "data" => self.frame_data.push(value.to_string()),
                    // `id`, `retry`, and unknown fields are ignored.
                    _ => {}
                }
            }
            let Some(event_name) = self.frame_event.take() else {
                self.frame_data.clear();
                continue;
            };
            let data = std::mem::take(&mut self.frame_data).join("\n");
            match DaemonEvent::from_sse(&event_name, &data) {
                Ok(Some(event)) => return Some(Ok(event)),
                // Unknown event name (newer daemon): skip silently.
                Ok(None) => continue,
                Err(error) => return Some(Err(ClientError::Json(error))),
            }
        }
    }
}

impl Stream for EventSubscription {
    type Item = Result<DaemonEvent, ClientError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            if let Some(item) = self.take_parsed_event() {
                return Poll::Ready(Some(item));
            }
            if self.byte_stream_done {
                return Poll::Ready(None);
            }
            match self.bytes.as_mut().poll_next(cx) {
                Poll::Ready(Some(Ok(chunk))) => self.buffer.extend_from_slice(&chunk),
                Poll::Ready(Some(Err(error))) => {
                    self.byte_stream_done = true;
                    return Poll::Ready(Some(Err(ClientError::Http(error))));
                }
                Poll::Ready(None) => self.byte_stream_done = true,
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::http::{HeaderMap, StatusCode, header};
    use axum::response::Response;
    use axum::routing::get;
    use sigillum_api::{
        EVENT_NAME_OPERATION, EVENT_NAME_SNAPSHOT, EVENT_NAME_STATUS, STATUS_EVENT_LOCKED,
    };

    use super::*;

    /// A canned SSE body exercising heartbeats, unknown event names, and
    /// every parser branch the daemon can emit.
    const CANNED_SSE: &str = concat!(
        "event: snapshot\n",
        "data: {\"v\":1,\"locked\":true,\"operations\":[]}\n",
        "\n",
        ":hb\n",
        "\n",
        "event: telemetry\n",
        "data: {\"future\":\"event kind\"}\n",
        "\n",
        "event: status\n",
        "data: {\"v\":1,\"kind\":\"locked\"}\n",
        "\n",
        "event: operation\n",
        "data: {\"v\":1,\"operation\":{\"id\":\"op-1\",\"kind\":\"inventory_scan_evm\",",
        "\"state\":\"running\",\"progress\":{\"processed\":0},\"created_at_unix\":1,",
        "\"updated_at_unix\":1}}\n",
        "\n",
    );

    async fn events_route(headers: HeaderMap) -> Response {
        let auth = headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        if auth != "Bearer test-token" {
            return Response::builder()
                .status(StatusCode::UNAUTHORIZED)
                .body("{\"code\":\"unauthorized\",\"error\":\"nope\"}".into())
                .unwrap();
        }
        Response::builder()
            .header(header::CONTENT_TYPE, "text/event-stream")
            .body(CANNED_SSE.into())
            .unwrap()
    }

    async fn spawn_events_server() -> std::net::SocketAddr {
        let app = Router::new().route("/api/events", get(events_route));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        addr
    }

    #[tokio::test]
    async fn subscription_parses_events_and_skips_unknown_kinds() {
        let addr = spawn_events_server().await;
        let client = SigillumClient::new(format!("http://{addr}")).expect("client builds");
        client.set_session_token("test-token");

        let mut subscription = client.subscribe_events().await.expect("subscription opens");

        let first = subscription.next().await.expect("stream open").unwrap();
        let DaemonEvent::Snapshot(snapshot) = first else {
            panic!("first event must be the snapshot: {first:?}");
        };
        assert!(snapshot.locked);
        assert_eq!(snapshot.v, 1);

        // The `telemetry` frame (unknown name) is skipped transparently.
        let second = subscription.next().await.expect("stream open").unwrap();
        let DaemonEvent::Status(status) = second else {
            panic!("expected status event: {second:?}");
        };
        assert_eq!(status.kind, STATUS_EVENT_LOCKED);

        let third = subscription.next().await.expect("stream open").unwrap();
        let DaemonEvent::Operation(operation) = third else {
            panic!("expected operation event: {third:?}");
        };
        assert_eq!(operation.operation.id, "op-1");

        // The server closes the body after the canned frames: stream ends.
        assert!(subscription.next().await.is_none());
    }

    #[tokio::test]
    async fn subscription_maps_401_to_api_error_and_clears_token() {
        let addr = spawn_events_server().await;
        let client = SigillumClient::new(format!("http://{addr}")).expect("client builds");
        client.set_session_token("wrong-token");

        let error = match client.subscribe_events().await {
            Ok(_) => panic!("subscribe must fail with 401"),
            Err(error) => error,
        };
        match error {
            ClientError::Api { status, code, .. } => {
                assert_eq!(status, StatusCode::UNAUTHORIZED);
                assert_eq!(code.as_deref(), Some("unauthorized"));
            }
            other => panic!("expected Api error, got {other:?}"),
        }
        assert_eq!(client.session_token(), None);
    }

    /// Byte-at-a-time chunking proves frames are reassembled across chunk
    /// boundaries exactly as they are on the network path.
    #[test]
    fn parser_splits_frames_across_chunk_boundaries() {
        struct ByteStream {
            bytes: std::vec::IntoIter<u8>,
        }
        impl Stream for ByteStream {
            type Item = Result<Bytes, reqwest::Error>;
            fn poll_next(
                mut self: Pin<&mut Self>,
                _: &mut Context<'_>,
            ) -> Poll<Option<Self::Item>> {
                Poll::Ready(self.bytes.next().map(|byte| Ok(Bytes::from(vec![byte]))))
            }
        }

        let mut subscription = EventSubscription::new(ByteStream {
            bytes: CANNED_SSE.as_bytes().to_vec().into_iter(),
        });

        let waker = std::task::Waker::noop();
        let mut cx = Context::from_waker(waker);
        let mut events = Vec::new();
        loop {
            match Pin::new(&mut subscription).poll_next(&mut cx) {
                Poll::Ready(Some(Ok(event))) => events.push(event),
                Poll::Ready(None) => break,
                other => panic!("unexpected poll result: {other:?}"),
            }
        }
        assert_eq!(events.len(), 3);
        assert!(matches!(events[0], DaemonEvent::Snapshot(_)));
        assert!(matches!(events[1], DaemonEvent::Status(_)));
        assert!(matches!(events[2], DaemonEvent::Operation(_)));
        assert_eq!(events[0].event_name(), EVENT_NAME_SNAPSHOT);
        assert_eq!(events[1].event_name(), EVENT_NAME_STATUS);
        assert_eq!(events[2].event_name(), EVENT_NAME_OPERATION);
    }
}
