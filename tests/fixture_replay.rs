//! A program's tests with no database server (spec, crow test --fixtures):
//! a scenario reads a Postgres of the test's own while `crow check
//! --record-fixtures` keeps what it answered; with the server stopped,
//! `crow test --fixtures` runs the tests from what was kept — the clock at
//! the recording's moment — in the interpreter and with the app compiled,
//! and a statement no fixture holds is named. A machine without Postgres's
//! binaries skips it.

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
    day = fmt[now[], time = 'date']
    return p['`len[book]` trades, first `book.sym[0]`, day `day`']

page['/', home]
serve[3000]

test the_book_shows_its_trades[]
    a = app[home]
    h = html[a]
    assert[contains[h, '2 trades, first AAPL, day 2026-01-01']]
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
        assert!(said.contains("1 block pass") && said.contains("the databases answered from .crow/fixtures"), "{:?}: {}", args, said);
    }
    // a statement nothing recorded is named, in both modes
    std::fs::write(app.join("main.co"), program(db.port, "qty")).unwrap();
    for args in [&["test", "--fixtures", "main.co"][..], &["test", "--fixtures", "--compiled", "main.co"][..]] {
        let said = crow(&app, args);
        assert!(said.contains("no fixture held `SELECT sym, qty FROM trades ORDER BY qty` — `crow check --record-fixtures` records it"), "{:?}: {}", args, said);
    }
}
