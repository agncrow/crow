use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// The words that hand a reader a failure's id in place of its reason — a
/// route's 500, a page's handler that stopped — under which the server's
/// log files the reason: a reason names what the program reaches, a
/// database's address or a file's path (Vijay, 2026-10-01).
pub const LOG_HAS_WHY: &str = "the server's log has why, as ";

/// A response's body is text, as a feed's put takes it (spec, Responses):
/// the words both modes refuse another kind with, `what` the kind as a
/// refusal names it (`an int`).
pub fn takes_text(form: &str, what: &str) -> String {
    let status = if form == "respond" { "status, " } else { "" };
    format!("{}[] takes text, got {}: write it into text, {}[{}'`…`']", form, what, form, status)
}

/// A route's handler that answered no response: the words the lowering
/// refuses it with, and the server's log says where the run met one.
pub fn not_a_response(handler: &str, what: &str) -> String {
    // (`json` of text parses it: the likeliest way to answer a parsed value)
    let parsed = if what == "a JSON value" { " — json[…] of text parses it, and json[…] of what it parsed answers it: json[json[text]]" } else { "" };
    format!("{}[] answers {}, not a response: a route's handler returns text[…], html[…], json[…], respond[…], redirect[…], bytes[…], msgpack[…], arrow[…] or stream[…]{}", handler, what, parsed)
}

/// `respond[status, body]` handed a status that is no int.
pub fn takes_status(what: &str) -> String {
    format!("respond[] takes an HTTP status, an int from 100 to 999, got {}: respond[404, 'no such thing']", what)
}

/// `arrow[frame]` handed another kind.
pub fn takes_frame(what: &str) -> String {
    format!("arrow[] takes a frame, got {}", what)
}

/// The status `respond` answers with, where HTTP has one (100 to 999): one
/// it has not is refused in the words both modes say, never moved to a
/// status near it (the interpreter had answered 1000 as 599, and the
/// compiled program 65736 as 200).
pub fn status(n: i64) -> Result<u16, String> {
    match u16::try_from(n) {
        Ok(s) if (100..=999).contains(&s) => Ok(s),
        _ => Err(format!("respond[] takes an HTTP status, from 100 to 999, got {}", n)),
    }
}

/// [`status`] in a compiled program: a status HTTP has not stops the
/// handler at `line`, as the interpreter's does.
pub fn status_at(n: i64, line: usize) -> u16 {
    status(n).unwrap_or_else(|why| crow_runtime::stops::halt(line, &why))
}

/// A failure's id: eight hex digits, random, so the one a reader quotes
/// finds its line in the log.
pub fn failure_id() -> String {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    crow_runtime::signin::random_hex(4).unwrap_or_else(|_| format!("{:08x}", NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst)))
}

/// What a view shows in place of a value that failed: `error: ` and the
/// words that hand the reader the failure's id, under which the server's
/// log has the reason — the reason names what the program reaches, a
/// database's address, a file's path, and a page's reader is told none of
/// it (Vijay, 2026-10-05, as for a handler that stopped). The id is the
/// same for the same reason (eight hex digits of a hash of it), so a page
/// that renders the failure on every tick is stable and the log says it
/// once per process. A derivation still being computed is `error:
/// loading`, as it was: no failure, and nothing to log.
pub fn failed_in_view(reason: &str) -> String {
    if reason == "loading" {
        return "error: loading".to_string();
    }
    let id = reason_id(reason);
    if first_time(&id) {
        eprintln!("crow live: a view showed a failed value as {}: {}", id, reason);
    }
    format!("error: {}{}", LOG_HAS_WHY, id)
}

/// A failure's id from its reason: eight hex digits of its FNV-1a hash.
pub fn reason_id(reason: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in reason.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{:08x}", (h ^ (h >> 32)) as u32)
}

/// Whether a failure's id is seen for the first time in this process: the
/// log has each reason once. (The set is let go when it has grown large.)
fn first_time(id: &str) -> bool {
    static SHOWN: Mutex<Option<std::collections::HashSet<String>>> = Mutex::new(None);
    let mut guard = SHOWN.lock().unwrap_or_else(|p| p.into_inner());
    let shown = guard.get_or_insert_with(std::collections::HashSet::new);
    if shown.len() > 4096 {
        shown.clear();
    }
    shown.insert(id.to_string())
}

/// Text with each failure's id read past, as `<id>`: an id is random, a
/// line in one run's log, so what compares two runs — the served pages of
/// both modes, a baseline's scenarios — agrees on everything but it.
pub fn ids_read_past(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(LOG_HAS_WHY) {
        let (head, tail) = rest.split_at(at + LOG_HAS_WHY.len());
        out.push_str(head);
        rest = tail;
        // (eight hex digits are eight bytes)
        if let Some(id) = tail.get(..8).filter(|id| id.bytes().all(|b| b.is_ascii_hexdigit())) {
            out.push_str("<id>");
            rest = &tail[id.len()..];
        }
    }
    out.push_str(rest);
    out
}

/// HTTP response returned by Crow route handlers.
///
/// Handlers return responses via helper functions:
/// ```text
/// route[app, '/hello', fn[req]
///   json[{message, 'hello'}]
/// ]
/// ```
///
/// Supports text, binary and streamed bodies. A feed takes priority, then
/// the binary body, when converting to an axum response.
#[derive(Debug, Clone)]
pub struct CrowResponse {
    /// HTTP status code (e.g. 200, 404, 500)
    pub status: u16,
    /// Response headers
    pub headers: HashMap<String, String>,
    /// Text response body (used by json, html, text, respond)
    pub body: String,
    /// Binary response body (used by msgpack, arrow, bytes)
    /// When Some, takes priority over `body` in response_to_axum.
    pub binary_body: Option<Vec<u8>>,
    /// A streamed body (`stream[feed]`): its chunks go to the reader as the
    /// feed puts them. When Some, takes priority over both bodies.
    pub feed: Option<Feed>,
}

/// A response shown (`log[json[m]]`): its status, as the interpreter shows
/// one (compiled, it had no form to show and did not build — crow check
/// --self).
impl crow_runtime::display::CrowDisplay for CrowResponse {
    fn crow_repr(&self) -> String {
        self.to_string()
    }
}

/// (and in text: `'v=`v`'` had not built)
impl std::fmt::Display for CrowResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "<response {}>", self.status)
    }
}

/// What a feed is handed: `put`, which sends a chunk and answers whether
/// the reader still listens.
pub type Put = fn(String) -> bool;

/// What a feed runs: handed put, it answers why it failed, if it did.
type FeedFn = Box<dyn FnOnce(Put) -> Result<(), String> + Send>;

/// A streamed response's feed (`stream[feed]`): its name, and what it runs
/// once, when the response is sent, on a thread of its own. Copies of the
/// response share it, and only the first run puts anything.
#[derive(Clone)]
pub struct Feed(Arc<Mutex<Option<(String, FeedFn)>>>);

impl std::fmt::Debug for Feed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Feed")
    }
}

/// Where a feed's chunks go: handed each, it answers whether the reader
/// still listens.
type Sink = Box<dyn FnMut(Vec<u8>) -> bool>;

thread_local! {
    /// Where this thread's feed puts its chunks, while it runs.
    static SINK: RefCell<Option<Sink>> = const { RefCell::new(None) };
}

impl Feed {
    /// Run the feed on this thread, each chunk it puts handed to `sink`,
    /// which answers whether the reader still listens. A feed that fails
    /// or stops ends the body there, and the log says why — the same words
    /// in both modes.
    pub fn run(&self, sink: impl FnMut(Vec<u8>) -> bool + 'static) {
        let feed = match self.0.lock() {
            Ok(mut held) => held.take(),
            Err(poisoned) => poisoned.into_inner().take(),
        };
        let Some((name, feed)) = feed else { return };
        SINK.with(|s| *s.borrow_mut() = Some(Box::new(sink)));
        let ended = crow_runtime::stops::guarded(|| feed(put));
        SINK.with(|s| *s.borrow_mut() = None);
        match ended {
            Ok(Ok(())) => {}
            Ok(Err(why)) | Err(why) => eprintln!("crow: {} stopped: {}", name, why),
        }
    }
}

/// A feed's put (the function `stream[feed]` hands `feed`): the chunk goes
/// to the reader, and the answer is whether the reader still listens —
/// false once it has gone, and outside a running feed.
pub fn put(chunk: String) -> bool {
    // the sink is taken while it runs, so a put it made would find none
    let taken = SINK.with(|s| s.borrow_mut().take());
    let Some(mut sink) = taken else { return false };
    let listening = sink(chunk.into_bytes());
    SINK.with(|s| *s.borrow_mut() = Some(sink));
    listening
}

/// `sys.binary[bytes, media]`: a response of the bytes, as the media type
/// names them (`application/msgpack` from the packer written in Crow).
pub fn binary_of(b: &crow_runtime::sys::Bytes, media: &str) -> CrowResponse {
    CrowResponse::binary(200, b.0.clone(), media)
}

impl CrowResponse {
    /// Create a response with the given status and text body.
    pub fn new(status: u16, body: impl Into<String>) -> Self {
        Self {
            status,
            headers: HashMap::new(),
            body: body.into(),
            binary_body: None,
            feed: None,
        }
    }

    /// Create a response with the given status and binary body.
    pub fn binary(status: u16, bytes: Vec<u8>, content_type: impl Into<String>) -> Self {
        let mut resp = Self::new(status, "");
        resp.binary_body = Some(bytes);
        resp.headers.insert("content-type".to_string(), content_type.into());
        resp
    }

    /// JSON response (status 200, content-type: application/json).
    /// Serializes the value to JSON.
    pub fn json(value: serde_json::Value) -> Self {
        let body = serde_json::to_string(&value).unwrap_or_else(|_| "null".to_string());
        let mut resp = Self::new(200, body);
        resp.headers.insert(
            "content-type".to_string(),
            "application/json".to_string(),
        );
        resp
    }

    /// Arrow IPC binary response (status 200, content-type: application/vnd.apache.arrow.stream).
    /// Takes pre-serialized Arrow IPC bytes.
    pub fn arrow_ipc(bytes: Vec<u8>) -> Self {
        Self::binary(200, bytes, "application/vnd.apache.arrow.stream")
    }

    /// `arrow[frame]`: the frame as an Arrow IPC stream. A frame Arrow
    /// cannot hold answers 500 with the reason.
    #[cfg(feature = "arrow_ipc")]
    pub fn arrow_of(frame: &crow_runtime::frame::CrowFrame) -> Self {
        match crow_runtime::frame::to_arrow_ipc(frame) {
            Ok(bytes) => Self::arrow_ipc(bytes),
            Err(e) => Self::respond(500, format!("arrow: {}", e)),
        }
    }

    /// `stream[feed]`: a text response (status 200) whose body is what the
    /// feed puts, each chunk sent as it is put. `name` is the feed's, which
    /// the log names if it fails (answering the reason) or stops.
    pub fn stream(name: &str, feed: impl FnOnce(Put) -> Result<(), String> + Send + 'static) -> Self {
        let mut resp = Self::new(200, "");
        resp.headers.insert("content-type".to_string(), "text/plain".to_string());
        let feed: FeedFn = Box::new(feed);
        resp.feed = Some(Feed(Arc::new(Mutex::new(Some((name.to_string(), feed))))));
        resp
    }

    /// HTML response (status 200, content-type: text/html).
    pub fn html(body: impl Into<String>) -> Self {
        let mut resp = Self::new(200, body);
        resp.headers
            .insert("content-type".to_string(), "text/html".to_string());
        resp
    }

    /// Plain text response (status 200, content-type: text/plain).
    pub fn text(body: impl Into<String>) -> Self {
        let mut resp = Self::new(200, body);
        resp.headers
            .insert("content-type".to_string(), "text/plain".to_string());
        resp
    }

    /// Response with a specific status code and body.
    pub fn respond(status: u16, body: impl Into<String>) -> Self {
        Self::new(status, body)
    }

    /// A failure's answer: 500, with the id the server's log files its
    /// reason under — the client is told no more (a reason names what the
    /// program reaches: a database's address, a file's path).
    pub fn stopped(id: &str) -> Self {
        Self::respond(500, format!("the request could not be answered — {}{}", LOG_HAS_WHY, id))
    }

    /// Redirect response (302 Found).
    pub fn redirect(url: impl Into<String>) -> Self {
        let mut resp = Self::new(302, "");
        resp.headers
            .insert("location".to_string(), url.into());
        resp
    }

    /// Set a header on the response. Returns self for chaining.
    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(key.into(), value.into());
        self
    }

    /// Check if response status is 2xx.
    pub fn is_ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

impl Default for CrowResponse {
    fn default() -> Self {
        Self::new(200, "")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A failure's id is random, so what compares two runs reads past it —
    /// a baseline recording a stopped handler's notice had changed on every
    /// `crow check`; text that is no id after the words is left alone
    /// A view shows a failed value as an id the log has, the same id for
    /// the same reason, and a derivation still loading as it was
    #[test]
    fn a_failed_value_in_a_view_tells_the_reader_an_id_the_log_has() {
        let shown = failed_in_view("cannot read 'secrets/db.txt': no such file");
        assert_eq!(ids_read_past(&shown), "error: the server's log has why, as <id>");
        assert!(!shown.contains("secrets"));
        assert_eq!(shown, failed_in_view("cannot read 'secrets/db.txt': no such file"));
        assert_ne!(shown, failed_in_view("cannot convert 'x' to int"));
        assert_eq!(reason_id("a").len(), 8);
        assert!(reason_id("a").bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(failed_in_view("loading"), "error: loading");
        // the log says a reason once
        assert!(first_time("ffffffff"));
        assert!(!first_time("ffffffff"));
    }

    #[test]
    fn a_failure_id_is_read_past() {
        let told = format!("apply stopped, and nothing changed — {}3f9a1c07 eq saved", LOG_HAS_WHY);
        assert_eq!(ids_read_past(&told), "apply stopped, and nothing changed — the server's log has why, as <id> eq saved");
        let two = format!("{w}0badcafe, {w}12345678.", w = LOG_HAS_WHY);
        assert_eq!(ids_read_past(&two), "the server's log has why, as <id>, the server's log has why, as <id>.");
        for left in [format!("{}not-hex!", LOG_HAS_WHY), format!("{}12ab", LOG_HAS_WHY), format!("{}é1234567", LOG_HAS_WHY), "no id here".to_string()] {
            assert_eq!(ids_read_past(&left), left);
        }
        assert!(failure_id().len() == 8 && failure_id().bytes().all(|b| b.is_ascii_hexdigit()));
    }

    #[test]
    fn test_response_json() {
        let resp = CrowResponse::json(serde_json::json!({"name": "alice"}));
        assert_eq!(resp.status, 200);
        assert_eq!(resp.headers.get("content-type").unwrap(), "application/json");
        assert!(resp.body.contains("alice"));
    }

    #[test]
    fn test_response_html() {
        let resp = CrowResponse::html("<h1>Hello</h1>");
        assert_eq!(resp.status, 200);
        assert_eq!(resp.headers.get("content-type").unwrap(), "text/html");
        assert_eq!(resp.body, "<h1>Hello</h1>");
    }

    #[test]
    fn test_response_text() {
        let resp = CrowResponse::text("hello");
        assert_eq!(resp.status, 200);
        assert_eq!(resp.headers.get("content-type").unwrap(), "text/plain");
        assert_eq!(resp.body, "hello");
    }

    #[test]
    fn test_response_redirect() {
        let resp = CrowResponse::redirect("https://example.com");
        assert_eq!(resp.status, 302);
        assert_eq!(resp.headers.get("location").unwrap(), "https://example.com");
    }

    #[test]
    fn test_response_respond() {
        let resp = CrowResponse::respond(404, "not found");
        assert_eq!(resp.status, 404);
        assert_eq!(resp.body, "not found");
    }

    #[test]
    fn test_response_is_ok() {
        assert!(CrowResponse::new(200, "").is_ok());
        assert!(CrowResponse::new(201, "").is_ok());
        assert!(!CrowResponse::new(404, "").is_ok());
        assert!(!CrowResponse::new(500, "").is_ok());
    }

    #[test]
    fn test_response_with_header() {
        let resp = CrowResponse::text("ok").with_header("x-custom", "value");
        assert_eq!(resp.headers.get("x-custom").unwrap(), "value");
    }

    #[test]
    fn test_response_binary() {
        let resp = CrowResponse::binary(200, vec![1, 2, 3], "application/octet-stream");
        assert_eq!(resp.status, 200);
        assert_eq!(resp.binary_body.as_ref().unwrap(), &[1, 2, 3]);
        assert_eq!(resp.headers.get("content-type").unwrap(), "application/octet-stream");
    }

    #[test]
    fn test_response_stream_puts_in_order() {
        let resp = CrowResponse::stream("chunks", |put| {
            for i in 0..3 {
                put(format!("chunk {}\n", i));
            }
            Ok(())
        });
        assert_eq!(resp.headers.get("content-type").unwrap(), "text/plain");
        let got = Arc::new(Mutex::new(Vec::new()));
        let into = got.clone();
        resp.feed.clone().unwrap().run(move |chunk| {
            into.lock().unwrap().extend(chunk);
            true
        });
        assert_eq!(String::from_utf8(got.lock().unwrap().clone()).unwrap(), "chunk 0\nchunk 1\nchunk 2\n");
        // a feed runs once: a copy of the response puts nothing more
        resp.feed.unwrap().run(|_| panic!("a feed ran twice"));
    }

    #[test]
    fn test_put_answers_whether_the_reader_listens() {
        // outside a feed, nobody listens
        assert!(!put("lost".to_string()));
        let answers = Arc::new(Mutex::new(Vec::new()));
        let seen = answers.clone();
        let resp = CrowResponse::stream("xs", move |put| {
            for _ in 0..3 {
                seen.lock().unwrap().push(put("x".to_string()));
            }
            Ok(())
        });
        let mut room = 2;
        resp.feed.unwrap().run(move |_| {
            room -= 1;
            room >= 0
        });
        assert_eq!(*answers.lock().unwrap(), vec![true, true, false]);
    }

    #[test]
    fn test_a_feed_that_fails_or_stops_ends_the_body_there() {
        let body = |resp: CrowResponse| {
            let got = Arc::new(Mutex::new(Vec::new()));
            let into = got.clone();
            resp.feed.unwrap().run(move |chunk| {
                into.lock().unwrap().extend(chunk);
                true
            });
            let got = got.lock().unwrap().clone();
            String::from_utf8(got).unwrap()
        };
        // a value that failed, and a stop: what was put before stays sent,
        // and the stop goes no further than the feed
        assert_eq!(body(CrowResponse::stream("broke", |put| {
            put("a".to_string());
            Err("no more rows".to_string())
        })), "a");
        assert_eq!(body(CrowResponse::stream("halted", |put| {
            put("b".to_string());
            crow_runtime::stops::halt(3, "index 9 is out of range for an array of 2 items")
        })), "b");
    }


    #[cfg(feature = "arrow_ipc")]
    #[test]
    fn test_arrow_of_writes_the_frame() {
        use crow_runtime::frame::{Column, CrowFrame};
        let frame = CrowFrame::new(vec!["n".to_string()], vec![Column::Int(vec![1, 2, 3])], vec![vec![true; 3]]);
        let resp = CrowResponse::arrow_of(&frame);
        assert_eq!(resp.status, 200);
        assert_eq!(resp.headers.get("content-type").unwrap(), "application/vnd.apache.arrow.stream");
        assert_eq!(resp.binary_body.unwrap(), crow_runtime::frame::to_arrow_ipc(&frame).unwrap());
    }

    #[test]
    fn test_response_arrow_ipc() {
        let resp = CrowResponse::arrow_ipc(vec![0x41, 0x52, 0x52, 0x4F, 0x57]);
        assert_eq!(resp.status, 200);
        assert_eq!(resp.headers.get("content-type").unwrap(), "application/vnd.apache.arrow.stream");
        assert!(resp.binary_body.is_some());
    }
}

