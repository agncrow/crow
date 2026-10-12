//! A program's tests with no database server (spec, crow test --fixtures):
//! a scenario reads a Postgres of the test's own while `crow check
//! --record-fixtures` keeps what it answered; with the server stopped,
//! `crow test --fixtures` runs the tests from what was kept — the clock at
//! the recording's moment — in the interpreter and with the app compiled,
//! and a statement no fixture holds is named. A machine without Postgres's
//! binaries skips it. A request's answer is kept and replayed the same way
//! (spec, Requests), against a server of the test's own.

use std::process::{Command, Stdio};

mod common;
use common::Scratch;

fn program(port: u16, order: &str) -> String {
    format!(
        "use web, ui, db

conn = connect['postgres://crow@127.0.0.1:{port}/postgres']

derive book
    return query[conn, 'SELECT sym, qty FROM trades ORDER BY {order}']

view home[]
    day = fmt[now[], style = 'date']
    return p['`len[book]` trades, first `book.sym[0]`, day `day`']

page['/', home]
serve[3000]

test the_book_shows_its_trades[]
    a = app[home]
    h = html[a]
    assert['2 trades, first AAPL, day 2026-01-01' in h]
"
    )
}

/// `crow` run in the app's directory: what it said, both streams.
fn crow(dir: &std::path::Path, args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_crow")).args(args).current_dir(dir).stdin(Stdio::null()).output().unwrap();
    format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))
}

#[test]
fn a_programs_tests_run_from_what_a_recording_kept() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    let Some(db) = Scratch::new(root.join("crow_pg_replay")) else { return };
    assert!(db.psql("CREATE TABLE trades (sym TEXT NOT NULL, qty INTEGER NOT NULL); INSERT INTO trades VALUES ('MSFT', 200), ('AAPL', 100)"), "the table is seeded");
    let app = root.join("crow_replay_app");
    let _ = std::fs::remove_dir_all(&app);
    std::fs::create_dir_all(&app).unwrap();
    std::fs::write(app.join("main.co"), program(db.port, "sym")).unwrap();
    // the recording reads the server, and keeps what it answered
    let said = crow(&app, &["check", "--record-fixtures", "main.co"]);
    assert!(said.contains("recorded SELECT sym, qty FROM trades ORDER BY sym"), "{}", said);
    db.stop();
    // with the server gone the tests cannot run as they are
    let plain = crow(&app, &["test", "main.co"]);
    assert!(plain.contains("failing"), "{}", plain);
    // from what was kept, in both modes
    for args in [&["test", "--fixtures", "main.co"][..], &["test", "--fixtures", "--compiled", "main.co"][..]] {
        let said = crow(&app, args);
        assert!(said.contains("1 block pass") && said.contains("answered from .crow/fixtures"), "{:?}: {}", args, said);
    }
    // a statement nothing recorded is named, in both modes
    std::fs::write(app.join("main.co"), program(db.port, "qty")).unwrap();
    for args in [&["test", "--fixtures", "main.co"][..], &["test", "--fixtures", "--compiled", "main.co"][..]] {
        let said = crow(&app, args);
        assert!(said.contains("no fixture held `SELECT sym, qty FROM trades ORDER BY qty` — `crow check --record-fixtures` records it"), "{:?}: {}", args, said);
    }
}

/// A request's program: a page showing what a server at `port` said to a
/// GET of `/{path}`.
fn asking(port: u16, path: &str) -> String {
    format!(
        "use web, ui

derive said
    r = request['http://127.0.0.1:{port}/{path}']
    return r.body

view home[]
    return p['the server said `said`']

page['/', home]
serve[3000]

test the_page_shows_what_the_server_said[]
    a = app[home]
    h = html[a]
    assert['the server said hello from the server' in h]
"
    )
}

/// A recording keeps what a request was answered, under the request as it
/// was made; with the server gone, `crow test --fixtures` answers it from
/// the fixture in both modes and sends nothing; a request the recording
/// never saw is named.
#[test]
fn a_request_is_answered_from_what_a_recording_kept() {
    use std::io::{Read, Write};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    let app = root.join("crow_replay_request_app");
    let _ = std::fs::remove_dir_all(&app);
    std::fs::create_dir_all(&app).unwrap();
    // a server of the test's own: it counts what it is asked, and once
    // stopped it closes without an answer
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let asked = Arc::new(AtomicUsize::new(0));
    let stopped = Arc::new(AtomicBool::new(false));
    {
        let (asked, stopped) = (asked.clone(), stopped.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut s) = stream else { break };
                asked.fetch_add(1, Ordering::SeqCst);
                if stopped.load(Ordering::SeqCst) {
                    continue;
                }
                let mut head = [0u8; 4096];
                let _ = s.read(&mut head);
                let body = "hello from the server";
                let _ = s.write_all(format!("HTTP/1.1 200 OK\r\ncontent-type: text/plain\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}", body.len(), body).as_bytes());
            }
        });
    }
    std::fs::write(app.join("main.co"), asking(port, "hello")).unwrap();
    // the recording asks the server, and keeps what it answered
    let said = crow(&app, &["check", "--record-fixtures", "main.co"]);
    assert!(said.contains(&format!("recorded GET http://127.0.0.1:{}/hello — 200, 21 bytes", port)), "{}", said);
    assert!(asked.load(Ordering::SeqCst) >= 1, "the recording asked the server");
    stopped.store(true, Ordering::SeqCst);
    // with the server gone the test cannot run as it is
    let plain = crow(&app, &["test", "main.co"]);
    assert!(plain.contains("failing"), "{}", plain);
    // from what was kept, in both modes, and nothing is sent
    let before = asked.load(Ordering::SeqCst);
    for args in [&["test", "--fixtures", "main.co"][..], &["test", "--fixtures", "--compiled", "main.co"][..]] {
        let said = crow(&app, args);
        assert!(said.contains("1 block pass") && said.contains("answered from .crow/fixtures"), "{:?}: {}", args, said);
    }
    assert_eq!(asked.load(Ordering::SeqCst), before, "a replay sends nothing");
    // a request nothing recorded is named, in both modes
    std::fs::write(app.join("main.co"), asking(port, "other")).unwrap();
    for args in [&["test", "--fixtures", "main.co"][..], &["test", "--fixtures", "--compiled", "main.co"][..]] {
        let said = crow(&app, args);
        assert!(said.contains(&format!("no fixture held `GET http://127.0.0.1:{}/other` — `crow check --record-fixtures` records it where the server is", port)), "{:?}: {}", args, said);
    }
    assert_eq!(asked.load(Ordering::SeqCst), before, "a miss sends nothing either");
}
