//! A database that goes away under a running program, and comes back
//! (docs/roadmap.md, the 1.0 bar: a lost connection): while it is gone each
//! query is an error value the program reads, never a crash; once it is
//! back the next query reconnects. In both modes, against a Postgres of the
//! test's own — a scratch cluster on a port of its own, never a server the
//! machine already runs. A machine without Postgres's binaries skips it.

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

mod common;
use common::Scratch;

const PROGRAM: &str = "use db

port = args[][0]
conn = connect['postgres://crow@127.0.0.1:`port`/postgres']
made = exec[conn, 'CREATE TABLE IF NOT EXISTS ticks (n INT)']
for i in range[120]
    w = exec[conn, 'INSERT INTO ticks VALUES (?)', [i]]
    q = query[conn, 'SELECT count(*) AS c FROM ticks']
    if w.ok and q.ok
        log['`i` ok']
    else
        log['`i` down: `w.reason`']
    sleep[100]
";

/// The program run in one mode while its database goes away and comes
/// back: what it said, line by line.
fn ride_out(mode: &[&str], db: &Scratch, dir: &std::path::Path) -> Vec<String> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_crow")).args(mode).arg("main.co").arg(db.port.to_string()).current_dir(dir).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    let out = child.stdout.take().unwrap();
    let mut lines = Vec::new();
    let mut stopped = false;
    let mut restarted = false;
    let by = Instant::now() + Duration::from_secs(900);
    for line in BufReader::new(out).lines().map_while(Result::ok) {
        assert!(Instant::now() < by, "{:?} ran too long", mode);
        if !stopped && line.starts_with("3 ok") {
            db.stop();
            stopped = true;
        } else if stopped && !restarted && line.contains("down") {
            // a few failures, then the database is back
            if lines.iter().filter(|l: &&String| l.contains("down")).count() >= 2 {
                db.start();
                restarted = true;
            }
        }
        lines.push(line);
    }
    let status = child.wait().unwrap();
    let mut err = String::new();
    let _ = std::io::Read::read_to_string(&mut child.stderr.take().unwrap(), &mut err);
    assert!(status.success(), "{:?} exited {:?}: {}", mode, status, err);
    lines
}

#[test]
fn a_program_rides_out_its_database_going_away() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    let Some(db) = Scratch::new(root.join("crow_pg_scratch")) else { return };
    let app = root.join("crow_lost_connection_app");
    std::fs::create_dir_all(&app).unwrap();
    std::fs::write(app.join("main.co"), PROGRAM).unwrap();
    for mode in [&["run", "-i"][..], &["run"][..]] {
        let said = ride_out(mode, &db, &app);
        let first_down = said.iter().position(|l| l.contains("down")).unwrap_or_else(|| panic!("{:?}: the outage was seen: {:?}", mode, said));
        let last_down = said.iter().rposition(|l| l.contains("down")).unwrap_or(first_down);
        assert!(said[..first_down].iter().any(|l| l.ends_with(" ok")), "{:?}: {:?}", mode, said);
        assert!(said[last_down..].iter().any(|l| l.ends_with(" ok")), "{:?}: the program reconnected: {:?}", mode, said);
        assert!(said[first_down].contains("does not answer"), "{:?}: the outage says what it is: {}", mode, said[first_down]);
        assert_eq!(said.len(), 120, "{:?}: every round ran", mode);
        // the next run starts from a server that is up
        if !db.running() {
            db.start();
        }
    }
}

// (a route's handler is handed its request alone, so it connects itself:
// every connect to one URL is one handle, and the handlers share its pool)
const SERVED: &str = "use web, db

fn count[req]
    dbport = args[][0]
    conn = connect['postgres://crow@127.0.0.1:`dbport`/postgres']
    q = query[conn, 'SELECT 1 AS one']
    return text['one: `q.one[0]`']

fn slow[req]
    sleep[3000]
    return text['slow done']

route['/count', count]
route['/slow', slow]
serve[3000]
";

/// A request written whole, and what came back: its status and body.
fn ask(port: u16, request: &str) -> (u16, String) {
    use std::io::{Read, Write};
    let Ok(mut s) = std::net::TcpStream::connect(("127.0.0.1", port)) else { return (0, String::new()) };
    let _ = s.set_read_timeout(Some(Duration::from_secs(20)));
    let _ = s.write_all(request.as_bytes());
    let mut out = String::new();
    let _ = s.read_to_string(&mut out);
    let status = out.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    let body = out.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
    (status, body)
}

fn get(port: u16, path: &str) -> (u16, String) {
    ask(port, &format!("GET {} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n", path))
}

struct Served(std::process::Child);

impl Drop for Served {
    /// `crow run` asked to stop hands it to the program it runs (a kill
    /// cannot be handed on, and left the compiled app serving its port)
    fn drop(&mut self) {
        let _ = Command::new("kill").arg(self.0.id().to_string()).status();
        let by = Instant::now() + Duration::from_secs(10);
        while Instant::now() < by && matches!(self.0.try_wait(), Ok(None)) {
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// A served app whose database goes away and comes back: every request is
/// answered — 500 and an id while the database is gone, the reason in the
/// server's log beside it, 200 once it is back — a slow handler holds up no
/// other, and a malformed request leaves the server answering. In both
/// modes.
#[test]
fn a_server_answers_through_its_database_going_away() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    let Some(db) = Scratch::new(root.join("crow_pg_scratch_served")) else { return };
    let app = root.join("crow_lost_connection_served");
    std::fs::create_dir_all(&app).unwrap();
    std::fs::write(app.join("main.co"), SERVED).unwrap();
    for (mode, port) in [(&["run", "-i"][..], 3491u16), (&["run"][..], 3492u16)] {
        if !db.running() {
            db.start();
        }
        // (the server's log, where a failure's reason is written)
        let log_path = app.join(format!("server_{}.log", port));
        let log = std::fs::File::create(&log_path).unwrap();
        let child = Command::new(env!("CARGO_BIN_EXE_crow")).args(mode).arg("main.co").arg(db.port.to_string()).current_dir(&app).env("CROW_PORT", port.to_string()).stdout(Stdio::null()).stderr(log).spawn().unwrap();
        let _served = Served(child);
        let by = Instant::now() + Duration::from_secs(900);
        while get(port, "/count").0 != 200 {
            assert!(Instant::now() < by, "{:?}: the app never answered", mode);
            std::thread::sleep(Duration::from_millis(250));
        }
        assert_eq!(get(port, "/count"), (200, "one: 1".to_string()), "{:?}", mode);
        // a slow handler holds up no other request
        let slow = std::thread::spawn(move || get(port, "/slow"));
        std::thread::sleep(Duration::from_millis(300));
        let started = Instant::now();
        assert_eq!(get(port, "/count").0, 200, "{:?}", mode);
        assert!(started.elapsed() < Duration::from_secs(2), "{:?}: a slow handler held another up", mode);
        assert_eq!(slow.join().unwrap(), (200, "slow done".to_string()), "{:?}", mode);
        // the database goes away: the route answers 500 and an id, the
        // server's log says why beside it, and the server stays up
        db.stop();
        let (status, body) = get(port, "/count");
        assert_eq!(status, 500, "{:?}: {}", mode, body);
        let id = body.trim().rsplit(' ').next().unwrap_or("").to_string();
        assert!(body.starts_with("the request could not be answered — the server's log has why, as ") && id.len() == 8, "{:?}: {}", mode, body);
        assert!(!body.contains("does not answer"), "{:?}: the reason is not the client's: {}", mode, body);
        let logged = std::fs::read_to_string(&log_path).unwrap_or_default();
        assert!(logged.lines().any(|l| l.contains(&format!("count stopped [{}]", id)) && l.contains("does not answer")), "{:?}: the log has the reason beside {}: {}", mode, id, logged);
        // a malformed request is refused, and the server goes on
        let (bad, _) = ask(port, "GET /count HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: nope\r\nConnection: close\r\n\r\n");
        assert_eq!(bad, 400, "{:?}", mode);
        // it comes back: the next request reaches it
        db.start();
        let by = Instant::now() + Duration::from_secs(30);
        loop {
            let (status, body) = get(port, "/count");
            if status == 200 {
                assert_eq!(body, "one: 1", "{:?}", mode);
                break;
            }
            assert!(Instant::now() < by, "{:?}: never reconnected: {} {}", mode, status, body);
            std::thread::sleep(Duration::from_millis(250));
        }
    }
}
