//! A reader sends headers with a URL (spec, Files, URLs and S3): `read`,
//! `load` and the rest take `headers = map[…]` — an API's key, a token —
//! and send them with the GET, in both modes. Without them the server's
//! refusal is the read's error; a file given headers is refused, since it
//! has none to send.

use std::io::{BufRead, BufReader, Write};
use std::process::Command;

/// A server that answers only a request carrying the key: a note, or a
/// table at a path ending `.csv`; one after another.
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
            let path = first.split_whitespace().nth(1).unwrap_or("/").to_string();
            let (status, body) = match (keyed, path.ends_with(".csv")) {
                (false, _) => ("401 Unauthorized", "no key".to_string()),
                (true, true) => ("200 OK", "sym,qty\nAAPL,100\nMSFT,200\n".to_string()),
                (true, false) => ("200 OK", "hello with a key".to_string()),
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
"
    )
}

#[test]
fn a_reader_sends_its_headers_in_both_modes() {
    let port = keyed_server();
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/crow_reader_headers_app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.co"), program(port)).unwrap();
    let said = format!(
        "hello with a key\n['AAPL', 'MSFT']\n[100, 200]\nfalse\ncannot read 'http://127.0.0.1:{}/note': the server answered 401 Unauthorized\ncannot read 'local.txt' with headers: headers go with an http:// or https:// URL\n",
        port
    );
    for mode in [&["run", "-i"][..], &["run"][..]] {
        let out = Command::new(env!("CARGO_BIN_EXE_crow")).args(mode).arg("main.co").current_dir(&dir).env("CROW_BUILD_PROFILE", "dev").output().unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), said, "{:?}: {}", mode, String::from_utf8_lossy(&out.stderr));
    }
}
