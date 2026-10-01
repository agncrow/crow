//! A reader sends headers with a URL (spec, Files, URLs and S3): `read`,
//! `load` and the rest take `headers = map[…]` — an API's key, a token —
//! and send them with the GET, in both modes, with a `query` encoded into
//! the URL and a `timeout` when told. Without the key the server's refusal
//! is the read's error; a file given options is refused, since it has none
//! to send.

use std::io::{BufRead, BufReader, Write};
use std::process::Command;

/// A server that answers only a request carrying the key: a note (saying
/// the query it was asked with), a table at a path ending `.csv`, numbers
/// at `/numbers.csv`; one after another.
fn keyed_server() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut first = String::new();
            let _ = reader.read_line(&mut first);
            let mut keyed = false;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
                if line.to_ascii_lowercase().starts_with("authorization: bearer s3cret") {
                    keyed = true;
                }
            }
            let target = first.split_whitespace().nth(1).unwrap_or("/").to_string();
            let (path, query) = target.split_once('?').unwrap_or((target.as_str(), ""));
            let (status, body) = match (keyed, path.ends_with(".csv")) {
                (false, _) => ("401 Unauthorized", "no key".to_string()),
                (true, true) if path == "/numbers.csv" => ("200 OK", format!("a,b\n1,2\n3,{}\n", if query.is_empty() { "4" } else { "5" })),
                (true, true) => ("200 OK", "sym,qty\nAAPL,100\nMSFT,200\n".to_string()),
                (true, false) if query.is_empty() => ("200 OK", "hello with a key".to_string()),
                (true, false) => ("200 OK", format!("hello with a key\nasked {}", query)),
            };
            let mut out = stream;
            let _ = write!(out, "HTTP/1.1 {}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", status, body.len(), body);
        }
    });
    port
}

fn program(port: u16) -> String {
    format!(
        "use frame

key = map[authorization = 'Bearer s3cret']
t = read['http://127.0.0.1:{port}/note', headers = key]
log[t]
df = load['http://127.0.0.1:{port}/trades.csv', headers = key]
log[df.sym]
log[df.qty]
denied = read['http://127.0.0.1:{port}/note']
log[denied.ok]
log[denied.reason]
file = read['local.txt', headers = key]
log[file.reason]
asked = read['http://127.0.0.1:{port}/note', headers = key, query = map[q = 'x y', page = '2']]
log[asked]
waited = load['http://127.0.0.1:{port}/trades.csv', headers = key, timeout = 5]
log[waited.qty]
"
    )
}

/// The readers a module names, given options.
fn qualified(port: u16) -> String {
    format!(
        "use io, math

key = map[authorization = 'Bearer s3cret']
rows = io.csv['http://127.0.0.1:{port}/trades.csv', headers = key, timeout = 5]
log[rows[1]['qty']]
said = io.lines['http://127.0.0.1:{port}/note', headers = key, query = map[q = 'x y']]
log[said]
m = math.load['http://127.0.0.1:{port}/numbers.csv', headers = key, query = map[n = '2']]
log[m]
odd = io.lines['http://127.0.0.1:{port}/note', headers = key, timeout = -1]
log[odd.reason]
"
    )
}

#[test]
fn a_reader_sends_its_headers_in_both_modes() {
    let port = keyed_server();
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/crow_reader_headers_app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.co"), program(port)).unwrap();
    std::fs::write(dir.join("qualified.co"), qualified(port)).unwrap();
    let said = format!(
        "hello with a key\n['AAPL', 'MSFT']\n[100, 200]\nfalse\ncannot read 'http://127.0.0.1:{port}/note': the server answered 401 Unauthorized\ncannot read 'local.txt' with headers, a query or a timeout: they go with an http:// or https:// URL\nhello with a key\nasked page=2&q=x%20y\n[100, 200]\n"
    );
    let said_qualified = "200\n['hello with a key', 'asked q=x%20y']\n[[1, 2], [3, 5]]\na timeout is a number of seconds, 0 or more — not -1\n";
    for mode in [&["run", "-i"][..], &["run"][..]] {
        for (file, said) in [("main.co", said.as_str()), ("qualified.co", said_qualified)] {
            let out = Command::new(env!("CARGO_BIN_EXE_crow")).args(mode).arg(file).current_dir(&dir).env("CROW_BUILD_PROFILE", "dev").output().unwrap();
            assert_eq!(String::from_utf8_lossy(&out.stdout), said, "{:?} {}: {}", mode, file, String::from_utf8_lossy(&out.stderr));
        }
    }
}
