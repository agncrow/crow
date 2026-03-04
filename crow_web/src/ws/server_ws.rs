//! Server-side WebSocket support using axum.
//!
//! Provides `CrowWsHandler` and `CrowWsSender` for handling
//! WebSocket connections on the server, and `make_ws_axum_handler`
//! to create an axum-compatible handler for WebSocket upgrade.

use std::sync::Arc;

use axum::{
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    response::Response,
};
use tokio::sync::mpsc;

/// Handler function type for server-side WebSocket.
///
/// Called for each text message received from the client.
/// The handler receives a `CrowWsSender` to reply and the message text.
///
/// ```text
/// ws[app, '/chat', fn[sock, msg]
///   send[sock, 'echo: {msg}']
/// ]
/// ```
pub type CrowWsHandler = Arc<dyn Fn(CrowWsSender, String) + Send + Sync>;

/// A sender handle for replying to WebSocket messages in server handlers.
///
/// Passed to `CrowWsHandler` callbacks. Call `.send(msg)` to send a
/// text message back to the connected client.
#[derive(Clone)]
pub struct CrowWsSender {
    tx: mpsc::UnboundedSender<String>,
}

impl CrowWsSender {
    /// Send a text message back to the client.
    pub fn send(&self, msg: &str) {
        self.tx.send(msg.to_string()).ok();
    }
}

/// Create an axum handler that upgrades HTTP to WebSocket.
///
/// Used internally by `CrowApp::ws_route` to register a WebSocket
/// endpoint on the server.
pub fn make_ws_axum_handler(
    handler: CrowWsHandler,
) -> impl Fn(
    WebSocketUpgrade,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Response> + Send>>
       + Clone
       + Send
       + Sync
       + 'static
{
    move |ws: WebSocketUpgrade| {
        let handler = handler.clone();
        Box::pin(async move { ws.on_upgrade(move |socket| handle_socket(socket, handler)) })
    }
}

/// Internal: run the WebSocket connection loop.
///
/// Receives messages from the client, calls the user's handler,
/// and forwards any replies from the handler back to the client.
async fn handle_socket(mut ws: WebSocket, handler: CrowWsHandler) {
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    let sender = CrowWsSender { tx };

    loop {
        tokio::select! {
            msg = ws.recv() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        handler(sender.clone(), text.to_string());
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(_)) => break,
                    _ => continue, // skip binary, ping, pong
                }
            }
            Some(reply) = rx.recv() => {
                if ws.send(Message::Text(reply.into())).await.is_err() {
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_crow_ws_sender_send() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let sender = CrowWsSender { tx };

        sender.send("hello");
        sender.send("world");

        assert_eq!(rx.try_recv().unwrap(), "hello");
        assert_eq!(rx.try_recv().unwrap(), "world");
    }

    #[test]
    fn test_crow_ws_sender_clone() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let sender = CrowWsSender { tx };
        let sender2 = sender.clone();

        sender.send("from original");
        sender2.send("from clone");

        assert_eq!(rx.try_recv().unwrap(), "from original");
        assert_eq!(rx.try_recv().unwrap(), "from clone");
    }

    #[test]
    fn test_crow_ws_sender_closed_channel() {
        let (tx, rx) = mpsc::unbounded_channel();
        let sender = CrowWsSender { tx };
        drop(rx); // close the receiver

        // send on closed channel should not panic
        sender.send("hello");
    }

    #[test]
    fn test_make_ws_axum_handler_returns_handler() {
        let handler: CrowWsHandler = Arc::new(|_sender, _msg| {});
        let _axum_handler = make_ws_axum_handler(handler);
        // Just verify it compiles and doesn't panic
    }
}

