//! `web.ws` — a WebSocket client: a live feed a program reads (a market's
//! trades), in both modes the same (spec, web, WebSocket).
//!
//! ```text
//! use web.ws
//!
//! s = socket['wss://feed.example.com/stocks']
//! send[s, '{"action": "subscribe", "params": "T.AAPL"}']
//! msg = recv[s, 5]
//! close[s]
//! ```
//!
//! A socket is a handle any number of values hold (a function handed it, a
//! loop): one connection behind a lock. Every failure is the call's error
//! value: a server that does not answer, one that closed, a wait that
//! passed its seconds.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

/// A socket's connection, shared by every value that holds it.
#[derive(Clone)]
pub struct CrowSocket {
    inner: Arc<Mutex<Option<WebSocket<MaybeTlsStream<std::net::TcpStream>>>>>,
    url: Arc<str>,
}

impl std::fmt::Debug for CrowSocket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "socket {}", self.url)
    }
}

/// `socket[url]`: a connection to a WebSocket server, `ws://` or `wss://`.
pub fn open(url: &str) -> Result<CrowSocket, String> {
    let (ws, _) = tungstenite::connect(url).map_err(|e| format!("cannot open a socket to {}: {}", url, e))?;
    Ok(CrowSocket { inner: Arc::new(Mutex::new(Some(ws))), url: Arc::from(url) })
}

fn stream_of(ws: &WebSocket<MaybeTlsStream<std::net::TcpStream>>) -> Option<&std::net::TcpStream> {
    match ws.get_ref() {
        MaybeTlsStream::Plain(s) => Some(s),
        MaybeTlsStream::Rustls(s) => Some(s.get_ref()),
        _ => None,
    }
}

impl CrowSocket {
    /// A socket that is closed: what a socket that failed to open holds, so
    /// every call on it answers that it is closed.
    pub fn closed() -> Self {
        CrowSocket { inner: Arc::new(Mutex::new(None)), url: Arc::from("") }
    }

    /// `send[s, text]`: a text message, sent.
    pub fn send(&self, text: &str) -> Result<(), String> {
        let mut guard = crow_runtime::lock(&self.inner);
        let ws = guard.as_mut().ok_or_else(|| "the socket is closed".to_string())?;
        ws.send(Message::Text(text.to_string().into())).map_err(|e| format!("cannot send: {}", e))
    }

    /// `recv[s]` / `recv[s, secs]`: the next text message — waiting for it,
    /// or at most `secs` seconds. A binary message is read as text; a ping
    /// is answered and passed over. A server that closed, and a wait that
    /// passed its seconds, are errors.
    pub fn recv(&self, secs: Option<f64>) -> Result<String, String> {
        let mut guard = crow_runtime::lock(&self.inner);
        let ws = guard.as_mut().ok_or_else(|| "the socket is closed".to_string())?;
        if let Some(s) = stream_of(ws) {
            let _ = s.set_read_timeout(secs.filter(|s| *s > 0.0).map(Duration::from_secs_f64));
        }
        loop {
            match ws.read() {
                Ok(Message::Text(t)) => return Ok(t.to_string()),
                Ok(Message::Binary(b)) => return Ok(String::from_utf8_lossy(&b).into_owned()),
                Ok(Message::Close(_)) => {
                    *guard = None;
                    return Err("closed".to_string());
                }
                Ok(_) => continue,
                Err(tungstenite::Error::Io(e)) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
                    return Err(format!("no message in {} s", secs.unwrap_or(0.0)));
                }
                Err(tungstenite::Error::ConnectionClosed) | Err(tungstenite::Error::AlreadyClosed) => {
                    *guard = None;
                    return Err("closed".to_string());
                }
                Err(e) => return Err(format!("cannot read: {}", e)),
            }
        }
    }

    /// `close[s]`: the connection closed, for every value that holds it.
    pub fn close(&self) {
        let mut guard = crow_runtime::lock(&self.inner);
        if let Some(mut ws) = guard.take() {
            let _ = ws.close(None);
            // the server's acknowledgement, briefly
            if let Some(s) = stream_of(&ws) {
                let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
            }
            while let Ok(m) = ws.read() {
                if matches!(m, Message::Close(_)) {
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A server that echoes what it is sent, then closes when told "bye"
    fn echo_server() -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut ws = tungstenite::accept(stream).unwrap();
            while let Ok(msg) = ws.read() {
                if let Message::Text(t) = msg {
                    if t.as_str() == "bye" {
                        let _ = ws.close(None);
                        while ws.read().is_ok() {}
                        return;
                    }
                    let _ = ws.send(Message::Text(format!("echo: {}", t).into()));
                }
            }
        });
        port
    }

    #[test]
    fn a_socket_sends_and_reads_until_the_server_closes() {
        let port = echo_server();
        let s = open(&format!("ws://127.0.0.1:{}/feed", port)).unwrap();
        s.send("hello").unwrap();
        assert_eq!(s.recv(Some(5.0)), Ok("echo: hello".to_string()));
        // nothing more comes: the wait passes its seconds
        assert_eq!(s.recv(Some(0.2)), Err("no message in 0.2 s".to_string()));
        s.send("bye").unwrap();
        assert_eq!(s.recv(Some(5.0)), Err("closed".to_string()));
        assert_eq!(s.send("again"), Err("the socket is closed".to_string()));
        s.close();
    }

    #[test]
    fn a_server_that_does_not_answer_is_the_open_s_error() {
        let err = open("ws://127.0.0.1:1/feed").unwrap_err();
        assert!(err.starts_with("cannot open a socket to ws://127.0.0.1:1/feed"), "{}", err);
    }
}
