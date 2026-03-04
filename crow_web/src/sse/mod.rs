//! `web.sse` module — Server-Sent Events.
//!
//! Provides a simple API for pushing events from the server to connected
//! clients over long-lived HTTP connections.
//!
//! ```text
//! use web
//! use web.sse
//!
//! app = web.new[]
//! sse[app, '/events', fn[sender]
//!   send[sender, 'hello']
//!   send[sender, 'world']
//! ]
//! serve[app, 3000]
//! ```

use std::convert::Infallible;
use std::pin::Pin;
use std::sync::Arc;

use axum::response::sse::{Event, KeepAlive, Sse};
use futures_util::Stream;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

// ============================================================
// Types
// ============================================================

/// A sender handle passed to SSE handler functions.
///
/// The handler receives this and calls `.send(data)` or `.event(name, data)`
/// to push events to the client.
#[derive(Clone)]
pub struct CrowSseSender {
    tx: mpsc::Sender<CrowSseEvent>,
}

/// An SSE event with optional name and data.
#[derive(Debug, Clone)]
pub struct CrowSseEvent {
    /// Event name (maps to SSE `event:` field). `None` means unnamed.
    pub name: Option<String>,
    /// Event data (maps to SSE `data:` field).
    pub data: String,
}

/// Handler function type for SSE routes.
///
/// Called once per client connection. The handler receives a `CrowSseSender`
/// and pushes events through it. When the handler returns (or the sender is
/// dropped), the SSE stream ends.
pub type CrowSseHandler = Arc<dyn Fn(CrowSseSender) + Send + Sync>;

// ============================================================
// CrowSseSender impl
// ============================================================

impl CrowSseSender {
    /// Send a data-only event (no event name).
    pub fn send(&self, data: &str) {
        self.tx
            .blocking_send(CrowSseEvent {
                name: None,
                data: data.to_string(),
            })
            .ok();
    }

    /// Send a named event (`event: <name>\ndata: <data>`).
    pub fn event(&self, name: &str, data: &str) {
        self.tx
            .blocking_send(CrowSseEvent {
                name: Some(name.to_string()),
                data: data.to_string(),
            })
            .ok();
    }
}

// ============================================================
// Axum handler adapter
// ============================================================

/// Concrete return type for the SSE axum handler.
pub type SseResponse =
    Sse<axum::response::sse::KeepAliveStream<Pin<Box<dyn Stream<Item = Result<Event, Infallible>> + Send>>>>;

/// Create an axum handler that serves an SSE stream.
///
/// Used internally by `CrowApp::sse_route`.
pub fn make_sse_axum_handler(
    handler: CrowSseHandler,
) -> impl Fn() -> Pin<Box<dyn std::future::Future<Output = SseResponse> + Send>>
       + Clone
       + Send
       + Sync
       + 'static
{
    move || {
        let handler = handler.clone();
        Box::pin(async move {
            let (tx, rx) = mpsc::channel::<CrowSseEvent>(256);
            let sender = CrowSseSender { tx };

            // Spawn the user's handler in a blocking task so it can
            // call .send() (which uses blocking_send) freely.
            tokio::task::spawn_blocking(move || {
                handler(sender);
            });

            // Convert the receiver into a Stream of axum SSE Events.
            let stream = ReceiverStream::new(rx);
            let event_stream: Pin<Box<dyn Stream<Item = Result<Event, Infallible>> + Send>> =
                Box::pin(futures_util::StreamExt::map(stream, |evt| {
                    let mut e = Event::default().data(evt.data);
                    if let Some(name) = evt.name {
                        e = e.event(name);
                    }
                    Ok::<_, Infallible>(e)
                }));

            Sse::new(event_stream).keep_alive(KeepAlive::default())
        })
    }
}

// ============================================================
// Tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_crow_sse_sender_send() {
        let (tx, mut rx) = mpsc::channel(16);
        let sender = CrowSseSender { tx };

        // Use spawn_blocking since sender.send() uses blocking_send
        tokio::task::spawn_blocking(move || {
            sender.send("hello");
            sender.event("update", "world");
        })
        .await
        .unwrap();

        let evt1 = rx.recv().await.unwrap();
        assert_eq!(evt1.data, "hello");
        assert!(evt1.name.is_none());

        let evt2 = rx.recv().await.unwrap();
        assert_eq!(evt2.data, "world");
        assert_eq!(evt2.name, Some("update".to_string()));
    }

    #[tokio::test]
    async fn test_crow_sse_sender_clone() {
        let (tx, mut rx) = mpsc::channel(16);
        let sender = CrowSseSender { tx };
        let sender2 = sender.clone();

        tokio::task::spawn_blocking(move || {
            sender.send("from original");
            sender2.send("from clone");
        })
        .await
        .unwrap();

        let evt1 = rx.recv().await.unwrap();
        assert_eq!(evt1.data, "from original");
        let evt2 = rx.recv().await.unwrap();
        assert_eq!(evt2.data, "from clone");
    }
}

