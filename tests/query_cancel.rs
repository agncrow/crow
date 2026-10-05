//! A statement a stopping program had in flight is stopped on its server
//! (docs/db-in-crow-design.md, §5): a served app's handler runs a slow
//! query, the program is told to stop, and the server's query ends then —
//! Postgres by a CancelRequest (a scratch cluster of the test's own,
//! `pg_stat_activity` read), ClickHouse by `KILL QUERY` with the query's
//! id (a server of the test's own, which records the kill). In both modes.
//! A machine without Postgres's binaries skips the Postgres half.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

mod common;
use common::Scratch;

const PG_APP: &str = "use web, db

fn slow[req]
    dbport = args[][0]
    conn = connect['postgres://crow@127.0.0.1:`dbport`/postgres']
    q = query[conn, 'SELECT pg_sleep(30) AS s']
    return text['done']

fn ping[req]
    return text['pong']

route['/slow', slow]
route['/ping', ping]
serve[3000]
";

const CH_APP: &str = "use web, db

fn slow[req]
    chport = args[][0]
    conn = connect['clickhouse://127.0.0.1:`chport`/stuff']
    q = query[conn, 'SELECT sleep(3) AS s']
    return text['done']

fn ping[req]
    return text['pong']

route['/slow', slow]
route['/ping', ping]
serve[3000]
";

fn get(port: u16, path: &str) -> (u16, String) {
    let Ok(mut s) = std::net::TcpStream::connect(("127.0.0.1", port)) else { return (0, String::new()) };
    let _ = s.set_read_timeout(Some(Duration::from_secs(60)));
    let _ = s.write_all(format!("GET {} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n", path).as_bytes());
    let mut out = String::new();
    let _ = s.read_to_string(&mut out);
    let status = out.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    let body = out.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
    (status, body)
}

/// A served app: started, waited for, and told to stop (SIGTERM, as
/// `crow stop` and a supervisor tell it).
struct Served(std::process::Child);

impl Served {
    fn start(mode: &[&str], dir: &std::path::Path, port: u16, arg: &str) -> Served {
        let log = std::fs::File::create(dir.join(format!("server_{}.log", port))).unwrap();
        let child = Command::new(env!("CARGO_BIN_EXE_crow")).args(mode).arg("main.co").arg(arg).current_dir(dir).env("CROW_PORT", port.to_string()).env("CROW_BUILD_PROFILE", "dev").stdout(Stdio::null()).stderr(log).spawn().unwrap();
        let by = Instant::now() + Duration::from_secs(900);
        while get(port, "/ping").0 != 200 {
            assert!(Instant::now() < by, "{:?}: the app never answered", mode);
            std::thread::sleep(Duration::from_millis(250));
        }
        Served(child)
    }

    fn stop(mut self) {
        let _ = Command::new("kill").arg(self.0.id().to_string()).status();
        let by = Instant::now() + Duration::from_secs(15);
        while Instant::now() < by && matches!(self.0.try_wait(), Ok(None)) {
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

impl Drop for Served {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

/// How many backends of the scratch cluster run a pg_sleep right now.
fn sleeping(db: &Scratch) -> usize {
    let out = Command::new("psql").args(["-h", "127.0.0.1", "-p", &db.port.to_string(), "-U", "crow", "-d", "postgres", "-tA", "-c", "SELECT count(*) FROM pg_stat_activity WHERE state = 'active' AND query LIKE '%pg_sleep%' AND pid <> pg_backend_pid()"]).output().unwrap();
    String::from_utf8_lossy(&out.stdout).trim().parse().unwrap_or(0)
}

#[test]
fn a_stopping_programs_postgres_query_is_cancelled_on_the_server() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    let Some(db) = Scratch::new(root.join("crow_cancel_pg_cluster")) else { return };
    let app = root.join("crow_cancel_pg_app");
    std::fs::create_dir_all(&app).unwrap();
    std::fs::write(app.join("main.co"), PG_APP).unwrap();
    for mode in [&["run", "-i"][..], &["run"][..]] {
        let port = free_port();
        let served = Served::start(mode, &app, port, &db.port.to_string());
        let asked = std::thread::spawn(move || get(port, "/slow"));
        let by = Instant::now() + Duration::from_secs(20);
        while sleeping(&db) == 0 {
            assert!(Instant::now() < by, "{:?}: the server never ran the query", mode);
            std::thread::sleep(Duration::from_millis(100));
        }
        let stopped_at = Instant::now();
        served.stop();
        // the backend's pg_sleep(30) ends with the CancelRequest, not thirty seconds on
        let by = Instant::now() + Duration::from_secs(5);
        while sleeping(&db) > 0 {
            assert!(Instant::now() < by, "{:?}: the query outlived the program", mode);
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(stopped_at.elapsed() < Duration::from_secs(8), "{:?}: the stop took {:?}", mode, stopped_at.elapsed());
        let _ = asked.join();
    }
    let _ = std::fs::remove_dir_all(&app);
}

/// A ClickHouse of the test's own: a ping, the sweep, a describe, a slow
/// query answered when killed (or after three seconds), and the kills it
/// was sent, by the query id each named.
fn serve_clickhouse(stream: std::net::TcpStream, killed: Arc<Mutex<Vec<String>>>, slow_ids: Arc<Mutex<Vec<String>>>) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut out = stream;
    loop {
        let mut first = String::new();
        if reader.read_line(&mut first).unwrap_or(0) == 0 {
            return;
        }
        let mut length = 0usize;
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                break;
            }
            if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                length = v.trim().parse().unwrap_or(0);
            }
        }
        let mut body = vec![0u8; length];
        let _ = reader.read_exact(&mut body);
        let path = first.split_whitespace().nth(1).unwrap_or("").to_string();
        let id = path.split("query_id=").nth(1).unwrap_or("").split('&').next().unwrap_or("").to_string();
        let sql = String::from_utf8_lossy(&body).to_string();
        let answer: Vec<u8> = if sql.starts_with("SELECT sleep(") {
            slow_ids.lock().unwrap().push(id.clone());
            let by = Instant::now() + Duration::from_secs(3);
            while Instant::now() < by && !killed.lock().unwrap().contains(&id) {
                std::thread::sleep(Duration::from_millis(20));
            }
            // RowBinaryWithNamesAndTypes: one column `s UInt8`, one row of 0
            vec![1, 1, b's', 5, b'U', b'I', b'n', b't', b'8', 0]
        } else if let Some(rest) = sql.strip_prefix("KILL QUERY WHERE query_id = '") {
            killed.lock().unwrap().push(rest.split('\'').next().unwrap_or("").to_string());
            Vec::new()
        } else if sql.starts_with("SELECT * FROM (") {
            b"s\nUInt8\n".to_vec()
        } else if sql.starts_with("SELECT 1") {
            b"1\n".to_vec()
        } else {
            Vec::new()
        };
        let _ = write!(out, "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: Keep-Alive\r\nKeep-Alive: timeout=30\r\n\r\n", answer.len());
        let _ = out.write_all(&answer);
        let _ = out.flush();
    }
}

#[test]
fn a_stopping_programs_clickhouse_query_is_killed_by_its_id() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let chport = listener.local_addr().unwrap().port();
    let killed: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let slow_ids: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    {
        let (killed, slow_ids) = (killed.clone(), slow_ids.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let (k, s) = (killed.clone(), slow_ids.clone());
                std::thread::spawn(move || serve_clickhouse(stream, k, s));
            }
        });
    }
    let app = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/crow_cancel_ch_app");
    std::fs::create_dir_all(&app).unwrap();
    std::fs::write(app.join("main.co"), CH_APP).unwrap();
    for mode in [&["run", "-i"][..], &["run"][..]] {
        let port = free_port();
        let served = Served::start(mode, &app, port, &chport.to_string());
        let before = slow_ids.lock().unwrap().len();
        let asked = std::thread::spawn(move || get(port, "/slow"));
        let by = Instant::now() + Duration::from_secs(20);
        while slow_ids.lock().unwrap().len() == before {
            assert!(Instant::now() < by, "{:?}: the server never got the query", mode);
            std::thread::sleep(Duration::from_millis(50));
        }
        let id = slow_ids.lock().unwrap()[before].clone();
        assert!(id.starts_with("crow-"), "{:?}: the query's id: {}", mode, id);
        served.stop();
        let by = Instant::now() + Duration::from_secs(5);
        while !killed.lock().unwrap().contains(&id) {
            assert!(Instant::now() < by, "{:?}: the query {} was not killed: {:?}", mode, id, killed.lock().unwrap());
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = asked.join();
    }
    let _ = std::fs::remove_dir_all(&app);
}
