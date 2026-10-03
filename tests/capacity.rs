//! How far one process goes (docs/roadmap.md, the 1.0 bar; the book's
//! deployment chapter): a compiled app's sessions and requests, measured.
//! A page load opens a session, which the process keeps for its grace
//! period, so loading the page `CROW_CAPACITY_SESSIONS` times (2,000
//! unless told) and reading the resident memory before and after gives
//! the memory a session costs; `CROW_CAPACITY_SOCKETS` tabs (200 unless
//! told) then open the page's socket and hold it, each a connected
//! session with its render, which gives what a tab costs while it is
//! open; then eight clients ask a route and the page as fast as they are
//! answered for `CROW_CAPACITY_SECS` seconds (20 unless told), which
//! gives the requests a second one process answers and how long each
//! waits. Run on purpose:
//!
//! ```text
//! cargo test --release --test capacity -- --ignored --nocapture
//! ```

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const PROGRAM: &str = "use web, ui, db

book = connect['sqlite://capacity.db']
made = exec[book, 'CREATE TABLE IF NOT EXISTS ticks (n INTEGER NOT NULL)']

fn count[req]
    conn = connect['sqlite://capacity.db']
    q = query[conn, 'SELECT count(*) AS n FROM ticks']
    return text['ticks: `q.n[0]`']

fn status[req]
    return json[map[ok = 'yes', path = req.path]]

clicks = 0

fn more[]
    clicks = clicks + 1

view home[]
    return stack[h1['Capacity'], p['`clicks` clicks'], button['+', click = more]]

route['/count', count]
route['/status', status]
page['/', home]
serve[3000]
";

struct Served(std::process::Child);

impl Drop for Served {
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

/// One request, whole: its status and its body.
fn fetch(port: u16, path: &str) -> (u16, String) {
    let Ok(mut s) = std::net::TcpStream::connect(("127.0.0.1", port)) else { return (0, String::new()) };
    let _ = s.set_read_timeout(Some(Duration::from_secs(20)));
    let _ = s.write_all(format!("GET {} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n", path).as_bytes());
    let mut out = String::new();
    let _ = s.read_to_string(&mut out);
    let status = out.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    let body = out.split_once("\r\n\r\n").map(|(_, b)| b.to_string()).unwrap_or_default();
    (status, body)
}

/// One request, whole: its status, or 0 when nothing answered.
fn get(port: u16, path: &str) -> u16 {
    fetch(port, path).0
}

/// The path the page's own script opens its socket at: `var path = '…'`,
/// or a base the tab's own path follows (`'/_ws' + location.pathname`).
fn socket_path(page: &str) -> Option<String> {
    let at = page.find("var path = '")? + "var path = '".len();
    let end = page[at..].find('\'')?;
    let base = page[at..at + end].to_string();
    let statement = &page[at..page[at..].find(';').map_or(page.len(), |k| at + k)];
    Some(if statement.contains("location.pathname") { format!("{}/", base) } else { base })
}

/// A tab's socket, opened and left open: the session it holds lives while
/// the socket does. Its first message — the render — is read, so the
/// session has sent what a tab is sent.
fn open_tab(port: u16, path: &str) -> Result<tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<std::net::TcpStream>>, String> {
    let (mut ws, _) = tungstenite::connect(format!("ws://127.0.0.1:{}{}", port, path)).map_err(|e| format!("open: {}", e))?;
    if let tungstenite::stream::MaybeTlsStream::Plain(s) = ws.get_ref() {
        let _ = s.set_read_timeout(Some(Duration::from_secs(10)));
    }
    ws.read().map_err(|e| format!("first message: {}", e))?;
    Ok(ws)
}

/// The resident memory of a process and of its children, in KB.
fn rss_kb(pid: u32) -> u64 {
    let mut pids = vec![pid.to_string()];
    if let Ok(o) = Command::new("pgrep").args(["-P", &pid.to_string()]).output() {
        pids.extend(String::from_utf8_lossy(&o.stdout).split_whitespace().map(|s| s.to_string()));
    }
    pids.iter()
        .filter_map(|p| Command::new("ps").args(["-o", "rss=", "-p", p]).output().ok())
        .filter_map(|o| String::from_utf8_lossy(&o.stdout).trim().parse::<u64>().ok())
        .sum()
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    sorted[((sorted.len() - 1) as f64 * p).round() as usize]
}

/// `clients` threads asking `path` as fast as they are answered for
/// `secs`: how many were answered, how many failed, and the latencies.
fn load(port: u16, path: &'static str, clients: usize, secs: u64) -> (u64, u64, Vec<f64>) {
    let stop = Arc::new(AtomicBool::new(false));
    let failed = Arc::new(AtomicU64::new(0));
    let answered = Arc::new(AtomicU64::new(0));
    let latencies: Arc<Mutex<Vec<f64>>> = Arc::new(Mutex::new(Vec::new()));
    let mut handles = Vec::new();
    for _ in 0..clients {
        let (stop, failed, answered, latencies) = (stop.clone(), failed.clone(), answered.clone(), latencies.clone());
        handles.push(std::thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                let started = Instant::now();
                let status = get(port, path);
                let ms = started.elapsed().as_secs_f64() * 1000.0;
                if status == 200 {
                    answered.fetch_add(1, Ordering::SeqCst);
                    latencies.lock().unwrap().push(ms);
                } else {
                    failed.fetch_add(1, Ordering::SeqCst);
                }
            }
        }));
    }
    std::thread::sleep(Duration::from_secs(secs));
    stop.store(true, Ordering::SeqCst);
    for h in handles {
        let _ = h.join();
    }
    let mut taken = latencies.lock().unwrap().clone();
    taken.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    (answered.load(Ordering::SeqCst), failed.load(Ordering::SeqCst), taken)
}

#[test]
#[ignore]
fn how_far_one_process_goes() {
    let sessions: usize = std::env::var("CROW_CAPACITY_SESSIONS").ok().and_then(|s| s.parse().ok()).unwrap_or(2000);
    let secs: u64 = std::env::var("CROW_CAPACITY_SECS").ok().and_then(|s| s.parse().ok()).unwrap_or(20);
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/crow_capacity_app");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.co"), PROGRAM).unwrap();
    let port: u16 = 3497;
    let child = Command::new(env!("CARGO_BIN_EXE_crow")).args(["run", "main.co"]).current_dir(&dir).env("CROW_PORT", port.to_string()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
    let pid = child.id();
    let _served = Served(child);
    let by = Instant::now() + Duration::from_secs(900);
    while get(port, "/status") != 200 {
        assert!(Instant::now() < by, "the app never answered");
        std::thread::sleep(Duration::from_millis(250));
    }
    // warm: the first page loads bring the pages' code in
    for _ in 0..20 {
        assert_eq!(get(port, "/"), 200);
    }
    std::thread::sleep(Duration::from_secs(1));
    let before = rss_kb(pid);
    let started = Instant::now();
    let (opened, failed, taken) = {
        let stop = Arc::new(AtomicBool::new(false));
        let opened = Arc::new(AtomicU64::new(0));
        let failed = Arc::new(AtomicU64::new(0));
        let latencies: Arc<Mutex<Vec<f64>>> = Arc::new(Mutex::new(Vec::new()));
        let mut handles = Vec::new();
        for _ in 0..8 {
            let (stop, opened, failed, latencies) = (stop.clone(), opened.clone(), failed.clone(), latencies.clone());
            handles.push(std::thread::spawn(move || {
                while !stop.load(Ordering::SeqCst) && opened.load(Ordering::SeqCst) < sessions as u64 {
                    let s = Instant::now();
                    if get(port, "/") == 200 {
                        opened.fetch_add(1, Ordering::SeqCst);
                        latencies.lock().unwrap().push(s.elapsed().as_secs_f64() * 1000.0);
                    } else {
                        failed.fetch_add(1, Ordering::SeqCst);
                    }
                }
            }));
        }
        for h in handles {
            let _ = h.join();
        }
        let mut taken = latencies.lock().unwrap().clone();
        taken.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        (opened.load(Ordering::SeqCst), failed.load(Ordering::SeqCst), taken)
    };
    let elapsed = started.elapsed().as_secs_f64();
    std::thread::sleep(Duration::from_secs(2));
    let after = rss_kb(pid);
    println!("sessions: {} opened in {:.1}s ({:.0} a second), {} failed, p50 {:.2} ms, p99 {:.2} ms", opened, elapsed, opened as f64 / elapsed, failed, percentile(&taken, 0.5), percentile(&taken, 0.99));
    println!("memory: {} KB before, {} KB after: {:.1} KB a session", before, after, (after.saturating_sub(before)) as f64 / opened.max(1) as f64);
    assert_eq!(failed, 0, "a page load failed");
    // tabs: sockets opened and held, each a connected session with its render
    let tabs: usize = std::env::var("CROW_CAPACITY_SOCKETS").ok().and_then(|s| s.parse().ok()).unwrap_or(200);
    let (status, page) = fetch(port, "/");
    assert_eq!(status, 200);
    let path = socket_path(&page).expect("the page's script names its socket's path");
    std::thread::sleep(Duration::from_secs(1));
    let before_tabs = rss_kb(pid);
    let started = Instant::now();
    let mut held = Vec::with_capacity(tabs);
    for _ in 0..tabs {
        match open_tab(port, &path) {
            Ok(ws) => held.push(ws),
            Err(e) => {
                println!("a tab's socket at {} did not open: {}", path, e);
                break;
            }
        }
    }
    let opened_tabs = held.len();
    std::thread::sleep(Duration::from_secs(2));
    let after_tabs = rss_kb(pid);
    println!(
        "tabs: {} sockets opened and held in {:.1}s, each sent its render; memory {} KB before, {} KB with them open: {:.1} KB a connected session",
        opened_tabs,
        started.elapsed().as_secs_f64(),
        before_tabs,
        after_tabs,
        (after_tabs.saturating_sub(before_tabs)) as f64 / opened_tabs.max(1) as f64
    );
    assert_eq!(opened_tabs, tabs, "a tab's socket did not open");
    for ws in &mut held {
        let _ = ws.close(None);
    }
    drop(held);
    std::thread::sleep(Duration::from_secs(1));
    for (path, what) in [("/status", "a route answering JSON"), ("/count", "a route that queries"), ("/", "a page load")] {
        let (answered, failed, taken) = load(port, path, 8, secs);
        println!("{} ({}): {:.0} a second, {} failed, p50 {:.2} ms, p99 {:.2} ms", what, path, answered as f64 / secs as f64, failed, percentile(&taken, 0.5), percentile(&taken, 0.99));
        assert_eq!(failed, 0, "{} failed", what);
    }
    println!("memory at the end: {} KB", rss_kb(pid));
}
