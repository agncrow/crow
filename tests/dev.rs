//! `crow dev` from the outside: the program served by the interpreter and
//! reloaded as its file changes, each tab's session kept and its dev panel
//! told each event; a change that breaks it leaves the last good program
//! serving, whose pages are told the refusal; and the program stops with
//! `crow dev`.

use std::io::{BufRead, Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// A page's markup, or nothing while no program answers.
fn get(port: u16, path: &str) -> String {
    let Ok(mut s) = std::net::TcpStream::connect(("127.0.0.1", port)) else { return String::new() };
    let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = write!(s, "GET {} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n", path, port);
    let mut out = String::new();
    let _ = s.read_to_string(&mut out);
    out
}

/// A POST from someone without the dev token.
fn post(port: u16, path: &str) -> String {
    let Ok(mut s) = std::net::TcpStream::connect(("127.0.0.1", port)) else { return String::new() };
    let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = write!(s, "POST {} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n", path, port);
    let mut out = String::new();
    let _ = s.read_to_string(&mut out);
    out
}

fn until(what: &str, f: impl Fn() -> bool) {
    let by = Instant::now() + Duration::from_secs(90);
    while !f() {
        assert!(Instant::now() < by, "{} never happened", what);
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// What a page's socket is told first, within a few seconds: the words the
/// page script reads.
fn socket_words(port: u16) -> Vec<String> {
    use tungstenite::client::IntoClientRequest;
    let first = get(port, "/");
    let token: String = first.split("pageToken = '").nth(1).unwrap_or("").chars().take_while(|c| c.is_ascii_hexdigit()).collect();
    let req = format!("ws://127.0.0.1:{}/_ws/?p={}", port, token).into_client_request().unwrap();
    let Ok((mut ws, _)) = tungstenite::connect(req) else { return Vec::new() };
    if let tungstenite::stream::MaybeTlsStream::Plain(s) = ws.get_mut() {
        let _ = s.set_read_timeout(Some(Duration::from_millis(200)));
    }
    let mut words = Vec::new();
    let by = Instant::now() + Duration::from_secs(4);
    while Instant::now() < by {
        if let Ok(tungstenite::Message::Text(t)) = ws.read() {
            words.push(t.to_string());
        }
    }
    words
}

type Socket = tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<std::net::TcpStream>>;

/// `crow dev`, stopped however the test ends (its program goes with it).
struct Dev(std::process::Child);

impl Drop for Dev {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// A page's socket, opened with what a tab sends: `p=` the page's token,
/// or `t=` a session to resume.
fn open_socket(port: u16, query: &str) -> Socket {
    use tungstenite::client::IntoClientRequest;
    let req = format!("ws://127.0.0.1:{}/_ws/?{}", port, query).into_client_request().unwrap();
    let (mut ws, _) = tungstenite::connect(req).unwrap();
    if let tungstenite::stream::MaybeTlsStream::Plain(s) = ws.get_mut() {
        let _ = s.set_read_timeout(Some(Duration::from_millis(200)));
    }
    ws
}

/// What a socket says until a word holds `want`, or ten seconds pass.
fn read_until(ws: &mut Socket, want: &str) -> Vec<String> {
    let mut words = Vec::new();
    let by = Instant::now() + Duration::from_secs(10);
    while Instant::now() < by {
        if let Ok(tungstenite::Message::Text(t)) = ws.read() {
            let done = t.contains(want);
            words.push(t.to_string());
            if done {
                break;
            }
        }
    }
    words
}

#[test]
fn crow_dev_reloads_a_program_as_its_file_changes() {
    let dir = std::env::temp_dir().join(format!("crow_dev_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let port = 3433;
    let program = |word: &str| format!("use web, ui\nn = 0\nfn inc[]\n    n = n + 1\nview home[]\n    return stack[p['{} `n`'], button['+', click = inc]]\nserve[{}]\npage['/', home]\n", word, port);
    std::fs::write(dir.join("app.co"), program("first")).unwrap();
    let mut dev = Dev(Command::new(env!("CARGO_BIN_EXE_crow"))
        .args(["dev", "app.co"])
        .current_dir(&dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap());
    // what crow dev says, kept for a failure's message
    let said = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    for pipe in [Box::new(dev.0.stdout.take().unwrap()) as Box<dyn Read + Send>, Box::new(dev.0.stderr.take().unwrap())] {
        let said = said.clone();
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(pipe).lines().map_while(Result::ok) {
                said.lock().unwrap().push_str(&format!("{}\n", line));
            }
        });
    }
    let log = || said.lock().unwrap().clone();
    until("the first program serving", || get(port, "/").contains("<p>first 0</p>"));
    // a tab's session: a click, then the change, then the tab back
    let page = get(port, "/");
    let token: String = page.split("pageToken = '").nth(1).unwrap_or("").chars().take_while(|c| c.is_ascii_hexdigit()).collect();
    let mut ws = open_socket(port, &format!("p={}", token));
    let said = read_until(&mut ws, "\"session\"");
    let tab = said
        .iter()
        .filter_map(|w| serde_json::from_str::<serde_json::Value>(w).ok())
        .find(|m| m["type"] == "session")
        .and_then(|m| m["token"].as_str().map(str::to_string))
        .unwrap_or_default();
    assert!(!tab.is_empty(), "the tab holds its session's token: {:?}", said);
    ws.send(tungstenite::Message::Text(serde_json::json!({ "event": "inc", "seq": 1 }).to_string().into())).unwrap();
    let after = read_until(&mut ws, "first 1");
    assert!(after.iter().any(|w| w.contains("first 1")), "the click lands: {:?}", after);
    // the dev panel hears it: the event, the fact it moved, the render
    let heard = read_until(&mut ws, "\"panel\"");
    assert!(heard.iter().any(|w| w.contains("\"changed\":{\"n\":[0,1]}") && w.contains("\"events\":[\"inc\"]") && w.contains("\"render_ms\"")), "the panel hears the click: {:?}", heard);
    drop(ws);
    std::fs::write(dir.join("app.co"), program("second")).unwrap();
    until("the change reloading it", || get(port, "/").contains("<p>second 0</p>"));
    let mut back = open_socket(port, &format!("t={}", tab));
    let resumed = read_until(&mut back, "second 1");
    assert!(resumed.iter().any(|w| w.contains("second 1")), "the tab's session outlives the reload: {:?}\n{}", resumed, log());
    drop(back);
    // a change that breaks it: the last good program keeps serving, and
    // its pages are told why
    std::fs::write(dir.join("app.co"), program("third").replace("stack[p[", "stack[pp[")).unwrap();
    until("the refusal", || log().contains("refused"));
    assert!(log().contains("app.co: line 6: undefined function 'pp'"), "{}", log());
    assert!(get(port, "/").contains("<p>second 0</p>"), "the last good program serves: {}", log());
    let words = socket_words(port);
    assert!(words.iter().any(|w| w.contains("\"type\":\"dev\"") && w.contains("app.co: line 6: undefined function 'pp'")), "a page is told the refusal: {:?}\n{}", words, log());
    // only the crow dev that started the program speaks to its pages
    assert!(post(port, "/_dev/reload").starts_with("HTTP/1.1 403"), "a word without the token is refused");
    std::fs::write(dir.join("app.co"), program("fourth")).unwrap();
    until("the mended change reloading it", || get(port, "/").contains("<p>fourth 0</p>"));
    // its program stops with it
    drop(dev);
    until("the program stopping with crow dev", || get(port, "/").is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}
