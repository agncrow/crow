//! The web families of `crow check --self`: programs that serve. A case is
//! a route's handler — its body the case's, its `return` the response — at
//! `/cN`, and the requests the check asks it; a batch of them is one
//! program that serves, served by every configuration on a port of its own
//! (`CROW_PORT`), each case asked its requests in turn, and each answer
//! compared as `crow test --served` compares one (`served_parity::ask`: the
//! status, the content type and location, the body, a failure's id read
//! past) — then held to the spec's answer by tests/self/reference.py.
//!
//! The routes family: every response (spec, Responses) with every value of
//! the catalog; every way a handler reads its request — its path's params,
//! its query, its method and path, its body as text and as JSON, a header,
//! a form, a multipart body's files; and what a route answers that is no
//! handler's value — a method it does not name, a path nothing answers, a
//! limit passed, a value that failed, a stop, a status HTTP cannot carry.

use super::*;

/// The responses a route answers with, by name (spec, Responses). Every
/// name web reserves is one of these or one of `NOT_RESPONSES`: a new one
/// is classified before the check runs (a unit test holds it).
pub(super) const RESPONSES: &[&str] = &["text", "html", "json", "msgpack", "arrow", "bytes", "respond", "redirect", "stream"];

/// The names web reserves that answer no route: the server's, a page's, a
/// test's, the modules' (auth, ws, mail).
#[cfg(test)]
const NOT_RESPONSES: &[&str] = &[
    "route", "serve", "sign", "verify", "app", "click", "elements", "change", "box", "fits", "style", "socket", "send", "recv", "close", "page", "smtp", "mail", "mails", "reader", "sign_in",
    "sign_out", "download", "downloads",
];

/// The route a served batch answers its run's token at, and the variable
/// that hands the server the token: a server that answers it is the
/// batch's own.
pub(super) const READY: &str = "/self-ready";
pub(super) const TOKEN: &str = "CROW_SELF_TOKEN";

/// A port for a served batch: from a counter below the range the system
/// hands out (49152 on), each run's counter starting at a place of its own,
/// and one some other process holds passed over. (A port the system handed
/// out, let go and then given to the server, had been handed to another
/// batch in between: its readiness reached the other's server.)
fn free_port() -> Result<u16, String> {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    const FIRST: u32 = 41000;
    const SPAN: u32 = 8000;
    let start = std::process::id().wrapping_mul(2654435761) % SPAN;
    for _ in 0..SPAN {
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let port = u16::try_from(FIRST + (start + n) % SPAN).map_err(|e| e.to_string())?;
        if std::net::TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return Ok(port);
        }
    }
    Err("no free port".to_string())
}

/// A route asked once, by GET.
fn asked_once() -> Route {
    Route { asks: vec![Ask::get("")], ..Route::default() }
}

/// A request with a body and its content type.
fn posted(tail: &str, kind: &str, body: &[u8]) -> Ask {
    Ask { method: "POST", tail: tail.to_string(), headers: vec![("Content-Type".to_string(), kind.to_string())], body: body.to_vec() }
}

/// The views family's cases: every element with every value of the catalog
/// as its child (and text that holds what HTML must escape), the attributes
/// that pass through to HTML with text of every kind, HTML's boolean ones
/// true and false, and the layout vocabulary's attributes over the values
/// its words take — each a view drawn as a page at `/cN`.
pub(super) fn view_cases(all: &[Val]) -> Vec<Case> {
    let page = || Route { asks: vec![Ask::get("")], page: true, ..Route::default() };
    let mut cases = Vec::new();
    // (text a page must escape, and text that looks like a tag)
    let escaped = [
        Val { name: "markup".into(), expr: "'a < b & \"c\" \\'d\\' > e'".into(), setup: Vec::new(), imports: Vec::new() },
        Val { name: "tagtext".into(), expr: "'<b>x</b>'".into(), setup: Vec::new(), imports: Vec::new() },
    ];
    let shown: Vec<&Val> = all.iter().chain(escaped.iter()).filter(|v| !matches!(v.name.as_str(), "sizes" | "groups" | "lists" | "maskcmp")).collect();
    for e in DRAWN {
        for v in &shown {
            let n = v.name.as_str();
            cases.push(Case::new(format!("view/{}/{}", e, n), &[v], vec![format!("return {}[{}]", e, v.expr)]).served(page()).referring(format!("view:{}", e), &[n]));
        }
    }
    // the attributes that pass through, each with text of every kind
    let texts = ["text", "text0", "textuni", "markup", "tagtext"];
    for a in PASSED {
        for t in texts {
            let Some(v) = all.iter().chain(escaped.iter()).find(|v| v.name == t) else { continue };
            cases.push(Case::new(format!("view-attr/{}/{}", a, t), &[v], vec![format!("return div['x', {} = {}]", a, v.expr)]).served(page()).referring(format!("view-attr:{}", a), &[t]));
        }
    }
    // a class joins a container's own, written or computed
    for t in texts {
        let Some(v) = all.iter().chain(escaped.iter()).find(|v| v.name == t) else { continue };
        cases.push(Case::new(format!("view-class/stack/{}", t), &[v], vec![format!("return stack['x', class = {}]", v.expr)]).served(page()).referring("view-class:stack", &[t]));
        cases.push(Case::new(format!("view-class/computed/{}", t), &[v], vec![format!("c = {}", v.expr), "return stack['x', class = c]".to_string()]).served(page()).referring("view-class:stack", &[t]));
    }
    // HTML's boolean attributes: present or absent
    for b in crow_runtime::layout::BOOL_ATTRS {
        for (truth, word) in [("true", "bool"), ("false", "boolf")] {
            cases.push(Case::new(format!("view-bool/{}/{}", b, word), &[], vec![format!("return button['x', {} = {}]", b, truth)]).served(page()).referring(format!("view-bool:{}", b), &[word]));
        }
    }
    // the layout vocabulary: each attribute over the words it may take
    // (one it does not take is refused, alike in both modes)
    for a in crow_runtime::layout::LAYOUT_ATTRS {
        for w in LAYOUT_WORDS {
            cases.push(Case::new(format!("view-layout/{}/{}", a, w.trim_matches('\'')), &[], vec![format!("return stack['x', {} = {}]", a, w)]).served(page()).referring(format!("view-layout:{}", a), &[]));
        }
    }
    cases
}

/// The elements the views family draws: the vocabulary's own and HTML's,
/// all but the head's (`title`, `meta`), the void ones (`input`, `img`,
/// `divider`), a frame's `table` and the program's own `widget`.
const DRAWN: &[&str] = &[
    "div", "span", "p", "h1", "h2", "h3", "h4", "h5", "h6", "button", "ul", "ol", "li", "form", "a", "nav", "header", "footer", "section", "tr", "td", "th", "thead",
    "tbody", "option", "textarea", "label", "pre", "code", "stack", "row", "grid", "overlay", "scroll", "details", "summary", "dropdown", "modal", "drawer", "toast",
];

/// The attributes that pass through to HTML as they are written.
const PASSED: &[&str] = &["title", "class", "id", "name", "href", "placeholder", "lang", "role"];

/// The words the layout attributes are tried with.
const LAYOUT_WORDS: &[&str] = &["'s'", "'m'", "'xl'", "'center'", "'between'", "'true'", "'2'", "'muted'", "'accent'", "'hug'", "'card'", "'nope'", "true", "2"];

/// The routes family's cases.
pub(super) fn route_cases(all: &[Val]) -> Vec<Case> {
    let mut cases = Vec::new();
    // every response with every value, asked once
    let shown: Vec<&Val> = all.iter().filter(|v| !matches!(v.name.as_str(), "sizes" | "groups" | "lists" | "maskcmp")).collect();
    for v in &shown {
        let n = v.name.as_str();
        for form in RESPONSES {
            let call = match *form {
                // (a status of its own, the value its body)
                "respond" => format!("respond[201, {}]", v.expr),
                // (a stream's feed puts the value's text, then a bar)
                "stream" => format!("stream[self_feed, {}]", v.expr),
                f => format!("{}[{}]", f, v.expr),
            };
            cases.push(Case::new(format!("route/{}/{}", form, n), &[v], vec![format!("return {}", call)]).served(asked_once()).referring(format!("route:{}", form), &[n]));
        }
    }

    // the request, read every way: each case answers what it read, as JSON
    // or as text, and is asked the requests that read differently
    let multipart = b"--crowcheck\r\nContent-Disposition: form-data; name=\"f\"\r\n\r\nfield value\r\n--crowcheck\r\nContent-Disposition: form-data; name=\"upload\"; filename=\"a.txt\"\r\nContent-Type: text/plain\r\n\r\nhello file\r\n--crowcheck--\r\n";
    let reads: Vec<(&str, &str, Route)> = vec![
        ("params", "return json[req.params]", Route { tail: "/:name/:n".into(), asks: vec![Ask::get("/ada/3"), Ask::get("/h%C3%A9llo%20w/0")], ..Route::default() }),
        ("query", "return json[req.query]", Route { asks: vec![Ask::get("?q=hello%20world&n=3&e="), Ask::get("?q=a%2Bb"), Ask::get("")], ..Route::default() }),
        ("method-path", "return json[map[m = req.method, p = req.path]]", Route { asks: vec![Ask::get(""), Ask::get("?x=1")], ..Route::default() }),
        ("body-text", "return text[req.body]", Route { method: Some("POST"), asks: vec![posted("", "text/plain", b"a nest"), posted("", "text/plain", "h\u{e9}llo".as_bytes()), posted("", "text/plain", b"")], ..Route::default() }),
        // (a parsed body answered as the JSON text it encodes: `json[j]`
        // parses j's text again, which is no response)
        ("body-json", "j = json[req.body]\nreturn text[encode[j]]", Route { method: Some("POST"), asks: vec![posted("", "application/json", br#"{"a": [1, 2], "b": "x"}"#), posted("", "application/json", b"{nope")], ..Route::default() }),
        // (a parsed value's field, read as the text a response takes: text,
        // a number, and none)
        (
            "body-field",
            "j = json[req.body]\nreturn text[j.b]",
            Route { method: Some("POST"), asks: vec![posted("", "application/json", br#"{"b": "x"}"#), posted("", "application/json", br#"{"b": 3}"#), posted("", "application/json", b"{}")], ..Route::default() },
        ),
        // (a status read from the request: one HTTP has, and one it has not)
        ("status-read", "return respond[int[req.query['s']], 'x']", Route { asks: vec![Ask::get("?s=404"), Ask::get("?s=1000"), Ask::get("?s=65736")], ..Route::default() }),
        // (a handler that answers on one path only: the other falls off its end)
        ("falls-off", "if req.method eq 'POST'\n    return text['posted']", Route { asks: vec![Ask::get("")], ..Route::default() }),
        (
            "header",
            "return text[req.headers['x-self']]",
            Route { asks: vec![Ask { headers: vec![("X-Self".to_string(), "hello".to_string())], ..Ask::get("") }, Ask::get("")], ..Route::default() },
        ),
        ("form", "return json[req.form]", Route { method: Some("POST"), asks: vec![posted("", "application/x-www-form-urlencoded", b"a=1&b=two+words&c=%C3%A9")], ..Route::default() }),
        (
            "files",
            "return json[map[n = str[len[req.files]], f = req.form['f'], name = req.files[0]['name'], size = req.files[0]['size']]]",
            Route { method: Some("POST"), asks: vec![posted("", "multipart/form-data; boundary=crowcheck", multipart)], ..Route::default() },
        ),
        ("method-named", "return text['made']", Route { method: Some("POST"), asks: vec![posted("", "text/plain", b"x"), Ask::get("")], ..Route::default() }),
        ("limit", "return text['ok']", Route { limit: Some(2), asks: vec![Ask::get(""), Ask::get(""), Ask::get("")], ..Route::default() }),
        ("nowhere", "return text['here']", Route { asks: vec![Ask::get("/nowhere/else")], ..Route::default() }),
        ("failed", "return text[int['x']]", asked_once()),
        ("failed-in-json", "return json[map[a = int['x'], b = '1']]", asked_once()),
        ("stopped", "xs = [1, 2]\nreturn text['`xs[9]`']", asked_once()),
        ("bad-status", "return respond[1000, 'x']", asked_once()),
    ];
    for (name, body, route) in reads {
        cases.push(Case::new(format!("route-read/{}", name), &[], body.lines().map(str::to_string).collect()).served(route).referring(format!("route-read:{}", name), &[]));
    }
    cases
}

/// A served batch under one configuration: its program served (the
/// interpreter on the file, or the built binary) on a port of its own, each
/// case's requests asked in turn — every answer as both modes must agree on
/// it, the answers to a case's requests parted by a line `—` — then the
/// server stopped. A server that never serves ends every case with why; one
/// that stops answering ends the case it was asking with what it said as
/// it went (a crash is that case's), and is started again for the next.
pub(super) fn serve_batch(exe: &Path, dir: &Path, file: &str, binary: Option<&Path>, config: &Config, cases: &[(usize, Case)]) -> Vec<Seen> {
    let run_dir = sandbox(dir, file, config.name);
    let mut seen: Vec<Seen> = Vec::with_capacity(cases.len());
    let mut server: Option<Server> = None;
    for (k, (_, c)) in cases.iter().enumerate() {
        if server.is_none() {
            match Server::start(exe, dir, file, binary, config, &run_dir) {
                Ok(s) => server = Some(s),
                Err(why) => {
                    // (a program that never serves: every case, alike)
                    seen.extend(cases[k..].iter().map(|_| Seen { out: String::new(), end: why.clone() }));
                    return seen;
                }
            }
        }
        let Some(s) = server.as_mut() else { break };
        let mut answers: Vec<String> = Vec::new();
        let mut end = String::new();
        let page = c.serve.as_ref().is_some_and(|r| r.page);
        for ask in c.serve.as_ref().map(|r| r.asks.as_slice()).unwrap_or(&[]) {
            match crate::served_parity::ask(s.port, ask.method, &format!("/c{}{}", k, ask.tail), &ask.headers, &ask.body) {
                Ok(a) if page => answers.push(page_root(&a)),
                Ok(a) => answers.push(a),
                Err(e) => {
                    end = match s.stopped() {
                        Some(said) => format!("crash: {}", said),
                        None => format!("no answer: {}", e),
                    };
                    break;
                }
            }
        }
        if !end.is_empty() {
            // (started again for the next case)
            if let Some(s) = server.take() {
                s.stop();
            }
        }
        seen.push(Seen { out: if answers.is_empty() { String::new() } else { format!("{}\n", answers.join("\n—\n")) }, end });
    }
    if let Some(s) = server {
        s.stop();
    }
    seen
}

/// A page's answer as the views family reads it: its status, then the
/// markup its view drew — what the page's root holds, the page's scripts,
/// styles and tokens around it read past. An answer that is no page (a
/// failure's 500) is itself.
fn page_root(answer: &str) -> String {
    const ROOT: &str = "<div id=\"crow-root\">";
    let Some(status) = answer.split(" | ").next() else { return answer.to_string() };
    let Some(at) = answer.find(ROOT) else { return answer.to_string() };
    let rest = &answer[at + ROOT.len()..];
    match rest.find("</div>\n<script>") {
        Some(end) => format!("{} | {}", status, &rest[..end]),
        None => answer.to_string(),
    }
}

/// A served batch's server: its process (the leader of its group, so what
/// it starts stops with it) and its port.
struct Server {
    child: std::process::Child,
    port: u16,
}

impl Server {
    /// The batch served on a port of its own: one some other process took
    /// between the choice and the server's bind is passed over for the next.
    fn start(exe: &Path, dir: &Path, file: &str, binary: Option<&Path>, config: &Config, run_dir: &Path) -> Result<Server, String> {
        let mut last = String::new();
        for _ in 0..3 {
            match Server::start_on(free_port()?, exe, dir, file, binary, config, run_dir) {
                Err(why) if why.contains("already in use") => last = why,
                other => return other,
            }
        }
        Err(last)
    }

    fn start_on(port: u16, exe: &Path, dir: &Path, file: &str, binary: Option<&Path>, config: &Config, run_dir: &Path) -> Result<Server, String> {
        use std::os::unix::process::CommandExt;
        let token = format!("{}-{}", std::process::id(), port);
        let mut cmd = match binary {
            Some(b) => case_command(b, run_dir, config, false),
            None => {
                let mut c = case_command(exe, run_dir, config, false);
                c.arg("run").arg("-i").arg(dir.join(file));
                c
            }
        };
        cmd.env("CROW_PORT", port.to_string()).env(TOKEN, &token).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped()).process_group(0);
        let child = cmd.spawn().map_err(|e| format!("cannot start: {}", e))?;
        let mut server = Server { child, port };
        // (an interpreter lowers the batch before it serves; the server is
        // the batch's when it answers the token)
        let by = Instant::now() + Duration::from_secs(90);
        loop {
            if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok()
                && crate::served_parity::ask(port, "GET", READY, &[], &[]).is_ok_and(|a| a.starts_with("HTTP/1.1 200") && a.ends_with(&format!("| {}", token)))
            {
                return Ok(server);
            }
            if let Some(said) = server.stopped() {
                return Err(format!("exited before serving: {}", said));
            }
            if Instant::now() > by {
                server.stop();
                return Err("never served".to_string());
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// What the server said as it went, where it has stopped: its last line
    /// to stderr (a panic's, a stop's).
    fn stopped(&mut self) -> Option<String> {
        match self.child.try_wait() {
            Ok(Some(status)) => {
                let mut err = String::new();
                if let Some(mut e) = self.child.stderr.take() {
                    let _ = std::io::Read::read_to_string(&mut e, &mut err);
                }
                Some(err.lines().rev().find(|l| !l.trim().is_empty()).map(str::to_string).unwrap_or_else(|| format!("{}", status)))
            }
            _ => None,
        }
    }

    fn stop(mut self) {
        // SAFETY: a signal to the process group the server leads
        unsafe {
            libc::kill(-(self.child.id() as i32), libc::SIGKILL);
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every name web reserves is a response the family asks, or one it
    /// says answers no route: a new one is classified before the check runs.
    #[test]
    fn every_name_web_reserves_is_classified() {
        for name in crate::lower_ir::WEB_RESERVED.iter().chain(crate::lower_ir::WEB_EXTRA_FUNCTIONS) {
            assert!(RESPONSES.contains(name) || NOT_RESPONSES.contains(name), "web reserves '{}': a response the routes family asks (RESPONSES), or one that answers no route (NOT_RESPONSES)", name);
        }
        for name in RESPONSES.iter().filter(|n| **n != "json") {
            assert!(crate::lower_ir::WEB_RESERVED.contains(name), "'{}' is no name web reserves", name);
        }
    }

    /// A page's answer is its status and the markup its root holds.
    #[test]
    fn a_page_is_read_as_its_root() {
        let answer = "HTTP/1.1 200 OK | content-type: text/html; charset=utf-8 | <!DOCTYPE html><html><head><style>x</style></head><body>\n<div id=\"crow-root\"><div><p>a &lt; b</p></div></div>\n<script>var t = '1f';</script></body></html>";
        assert_eq!(page_root(answer), "HTTP/1.1 200 OK | <div><p>a &lt; b</p></div>");
        let failed = "HTTP/1.1 500 Internal Server Error | content-type: text/plain | the request could not be answered";
        assert_eq!(page_root(failed), failed);
    }

    /// A view's case is a view drawn as a page; a served batch of them is
    /// views, their pages and a server.
    #[test]
    fn a_view_case_is_a_page() {
        let cases = view_cases(&values());
        let picked: Vec<&Case> = cases.iter().filter(|c| c.id == "view/p/markup" || c.id == "view-bool/disabled/bool").collect();
        assert_eq!(picked.len(), 2);
        let (src, _) = program(&picked);
        assert!(src.starts_with("use ui, web\n"), "{}", src);
        assert!(src.contains("view self_case_0[]\n    return p['a < b & \"c\" \\'d\\' > e']\n"), "{}", src);
        assert!(src.contains("page['/c1', self_case_1]\n"), "{}", src);
    }

    /// A served batch is a program of handlers, their routes and a server.
    #[test]
    fn a_served_batch_is_handlers_routes_and_a_server() {
        let cases = route_cases(&values());
        let picked: Vec<&Case> = cases.iter().filter(|c| c.id == "route/json/int" || c.id == "route-read/params" || c.id == "route-read/limit").collect();
        assert_eq!(picked.len(), 3);
        let (src, _) = program(&picked);
        assert!(src.starts_with("use web\n"), "{}", src);
        assert!(src.contains("fn self_case_0[req]\n    return json[3]\n"), "{}", src);
        assert!(src.contains("route['/c1/:name/:n', self_case_1]") && src.contains("route['/c2', self_case_2, limit = 2]"), "{}", src);
        assert!(src.trim_end().ends_with("route['/self-ready', self_ready]\nserve[3000]"), "{}", src);
        assert!(src.contains("fn self_ready[req]\n    return respond[200, env['CROW_SELF_TOKEN', '']]\n"), "{}", src);
        assert!(!src.contains("CROW_FROM"), "{}", src);
    }
}
