//! `web.ws` module — WebSocket support.
//!
//! **Client mode** (standalone, no server needed):
//! ```text
//! use web.ws
//! sock = connect['ws://localhost:8080/feed']
//! send[sock, 'hello']
//! msg = recv[sock]
//! close[sock]
//! ```
//!
//! **Server mode** (with `http` feature):
//! ```text
//! use web
//! use web.ws
//! app = web.new[]
//! ws[app, '/chat', fn[sock, msg]
//!   send[sock, 'echo: {msg}']
//! ]
//! serve[app, 3000]
//! ```

use std::sync::Mutex;

use tungstenite::{connect as ws_connect, Message};

// ============================================================
// Client-side WebSocket (sync, uses tungstenite)
// ============================================================

/// A client-side WebSocket connection.
///
/// Wraps a tungstenite `WebSocket`. If the connection failed,
/// `.ok()` returns `false` and all operations are no-ops.
pub struct CrowWebSocket {
    inner: Option<Mutex<tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<std::net::TcpStream>>>>,
}

impl CrowWebSocket {
    /// True if the connection is open.
    pub fn ok(&self) -> bool {
        self.inner.is_some()
    }

    /// Send a text message.
    pub fn send(&self, msg: &str) {
        if let Some(ref socket) = self.inner {
            if let Ok(mut s) = socket.lock() {
                s.send(Message::Text(msg.into())).ok();
            }
        }
    }

    /// Receive a text message (blocks until one arrives).
    /// Returns None on error or connection close.
    pub fn recv(&self) -> Option<String> {
        if let Some(ref socket) = self.inner {
            if let Ok(mut s) = socket.lock() {
                loop {
                    match s.read() {
                        Ok(Message::Text(text)) => return Some(text.to_string()),
                        Ok(Message::Close(_)) => return None,
                        Ok(_) => continue, // skip binary, ping, pong
                        Err(_) => return None,
                    }
                }
            } else {
                None
            }
        } else {
            None
        }
    }

    /// Close the connection.
    pub fn close(&self) {
        if let Some(ref socket) = self.inner {
            if let Ok(mut s) = socket.lock() {
                s.close(None).ok();
                // Drain remaining messages until close is acknowledged
                loop {
                    match s.read() {
                        Ok(Message::Close(_)) | Err(_) => break,
                        _ => continue,
                    }
                }
            }
        }
    }
}

/// Connect to a WebSocket server (client mode).
///
/// Returns a `CrowWebSocket`. Check `.ok()` to see if the connection
/// succeeded. On failure, `.ok()` is `false` and all operations are no-ops.
pub fn connect(url: &str) -> CrowWebSocket {
    match ws_connect(url) {
        Ok((socket, _response)) => CrowWebSocket {
            inner: Some(Mutex::new(socket)),
        },
        Err(_) => CrowWebSocket { inner: None },
    }
}

// ============================================================
// Tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_connect_invalid_url_returns_not_ok() {
        let sock = connect("ws://127.0.0.1:1"); // port 1 — should fail
        assert!(!sock.ok());
    }

    #[test]
    fn test_send_on_failed_socket_is_noop() {
        let sock = connect("ws://127.0.0.1:1");
        sock.send("hello"); // should not panic
    }

    #[test]
    fn test_recv_on_failed_socket_returns_none() {
        let sock = connect("ws://127.0.0.1:1");
        assert!(sock.recv().is_none());
    }

    #[test]
    fn test_close_on_failed_socket_is_noop() {
        let sock = connect("ws://127.0.0.1:1");
        sock.close(); // should not panic
    }

    /// A server that echoes what it is sent: the client connects, sends,
    /// reads the echo, and closes
    #[test]
    fn the_client_talks_to_a_server() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut ws = tungstenite::accept(stream).unwrap();
            while let Ok(msg) = ws.read() {
                if let Message::Text(t) = msg {
                    let _ = ws.send(Message::Text(format!("echo: {}", t).into()));
                }
            }
        });
        let sock = connect(&format!("ws://127.0.0.1:{}/echo", port));
        assert!(sock.ok(), "the connection opens");
        sock.send("hello");
        assert_eq!(sock.recv(), Some("echo: hello".to_string()));
        sock.close();
    }
}

