//! A served app under load for a while (docs/roadmap.md, the 1.0 bar: a
//! soak): its memory and its latency steady, every request answered. A
//! compiled app with a route that queries a database, a route that answers
//! JSON and a live page, asked by a few clients at once for
//! `CROW_SOAK_SECS` seconds (two minutes unless told; a day is
//! `CROW_SOAK_SECS=86400`). Its resident memory and its latency are sampled
//! every few seconds and printed, and after the first quarter — the warm-up
//! — the memory may grow only a little and the slow requests may slow only a
//! little. Run on purpose:
//!
//! ```text
//! CROW_SOAK_SECS=600 cargo test --release --test soak -- --ignored --nocapture
//! ```

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const PROGRAM: &str = "use web, ui, db

book = connect['sqlite://soak.db']
made = query[book, 'CREATE TABLE IF NOT EXISTS ticks (n INTEGER NOT NULL)']

fn count[req]
    conn = connect['sqlite://soak.db']
    q = query[conn, 'SELECT count(*) AS n FROM ticks']
    return text['ticks: `q.n[0]`']

fn status[req]
    return json[map[ok = 'yes', path = req.path]]

clicks = 0

fn more[]
    clicks = clicks + 1

view home[]
    return stack[p['`clicks` clicks'], button['+', click = more]]

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

/// One request, whole: its status, or 0 when nothing answered.
fn get(port: u16, path: &str) -> u16 {
    let Ok(mut s) = std::net::TcpStream::connect(("127.0.0.1", port)) else { return 0 };
    let _ = s.set_read_timeout(Some(Duration::from_secs(20)));
    let _ = s.write_all(format!("GET {} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n", path).as_bytes());
    let mut out = String::new();
    let _ = s.read_to_string(&mut out);
    out.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0)
}

/// The resident memory of a process and of its children, in KB (`crow
/// run` starts the built program as a child).
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

#[test]
#[ignore]
fn an_app_under_load_stays_steady() {
    let secs: u64 = std::env::var("CROW_SOAK_SECS").ok().and_then(|s| s.parse().ok()).unwrap_or(120);
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/crow_soak_app");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.co"), PROGRAM).unwrap();
    let port: u16 = 3493;
    let child = Command::new(env!("CARGO_BIN_EXE_crow")).args(["run", "main.co"]).current_dir(&dir).env("CROW_PORT", port.to_string()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
    let pid = child.id();
    let _served = Served(child);
    let by = Instant::now() + Duration::from_secs(900);
    while get(port, "/status") != 200 {
        assert!(Instant::now() < by, "the app never answered");
        std::thread::sleep(Duration::from_millis(250));
    }
    let stop = Arc::new(AtomicBool::new(false));
    let failed = Arc::new(AtomicU64::new(0));
    let answered = Arc::new(AtomicU64::new(0));
    let latencies: Arc<Mutex<Vec<f64>>> = Arc::new(Mutex::new(Vec::new()));
    let mut clients = Vec::new();
    for c in 0..4 {
        let (stop, failed, answered, latencies) = (stop.clone(), failed.clone(), answered.clone(), latencies.clone());
        clients.push(std::thread::spawn(move || {
            let paths = ["/count", "/status", "/"];
            let mut k = c;
            while !stop.load(Ordering::SeqCst) {
                let started = Instant::now();
                let status = get(port, paths[k % paths.len()]);
                let ms = started.elapsed().as_secs_f64() * 1000.0;
                k += 1;
                if status == 200 {
                    answered.fetch_add(1, Ordering::SeqCst);
                    latencies.lock().unwrap().push(ms);
                } else {
                    failed.fetch_add(1, Ordering::SeqCst);
                }
                // about a hundred requests a second in all: a soak is a
                // steady load, and a connection per request leaves its
                // socket waiting a while after it closes
                std::thread::sleep(Duration::from_millis(40));
            }
        }));
    }
    // samples: (seconds in, rss KB, p50 ms, p99 ms, requests)
    let mut samples: Vec<(u64, u64, f64, f64, usize)> = Vec::new();
    let start = Instant::now();
    let every = Duration::from_secs(5.min(secs.max(1)));
    while start.elapsed() < Duration::from_secs(secs) {
        std::thread::sleep(every);
        let mut taken: Vec<f64> = std::mem::take(&mut *latencies.lock().unwrap());
        taken.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let s = (start.elapsed().as_secs(), rss_kb(pid), percentile(&taken, 0.5), percentile(&taken, 0.99), taken.len());
        println!("{:>6}s  rss {:>7} KB  p50 {:>7.2} ms  p99 {:>7.2} ms  {:>6} requests", s.0, s.1, s.2, s.3, s.4);
        samples.push(s);
    }
    stop.store(true, Ordering::SeqCst);
    for c in clients {
        let _ = c.join();
    }
    let failed = failed.load(Ordering::SeqCst);
    println!("{} answered, {} not", answered.load(Ordering::SeqCst), failed);
    assert_eq!(failed, 0, "every request is answered");
    assert!(samples.len() >= 4, "enough samples to judge: {}", samples.len());
    // after the warm-up: the memory steady, the slow requests steady
    let warm = samples.len() / 4;
    let base_rss = samples[warm].1 as f64;
    let last_rss = samples.last().unwrap().1 as f64;
    assert!(last_rss <= base_rss * 1.25 + 30_000.0, "memory grew from {} KB to {} KB", base_rss, last_rss);
    // the memory's trend after the warm-up, as a line through the samples:
    // what it would grow by in a day. Judged on five minutes or more — a
    // shorter run's line is the allocator settling. (The list of sessions
    // the app can wake had kept every session a page load ended: some 7 KB
    // a second at this load.)
    let after = &samples[warm..];
    let n = after.len() as f64;
    let (mx, my) = (after.iter().map(|s| s.0 as f64).sum::<f64>() / n, after.iter().map(|s| s.1 as f64).sum::<f64>() / n);
    let cov: f64 = after.iter().map(|s| (s.0 as f64 - mx) * (s.1 as f64 - my)).sum();
    let var: f64 = after.iter().map(|s| (s.0 as f64 - mx).powi(2)).sum();
    let kb_per_sec = if var > 0.0 { cov / var } else { 0.0 };
    println!("memory trend after the warm-up: {:.2} KB/s, {:.0} MB a day", kb_per_sec, kb_per_sec * 86_400.0 / 1024.0);
    let window = after.last().unwrap().0 - after[0].0;
    if window >= 300 {
        assert!(kb_per_sec * 86_400.0 / 1024.0 <= 256.0, "the memory grows {:.2} KB a second, {:.0} MB a day", kb_per_sec, kb_per_sec * 86_400.0 / 1024.0);
    }
    let base_p99 = samples[warm..].iter().map(|s| s.3).fold(f64::INFINITY, f64::min);
    let last_p99 = samples.last().unwrap().3;
    assert!(last_p99 <= base_p99 * 10.0 + 50.0, "the slow requests slowed from {:.2} ms to {:.2} ms", base_p99, last_p99);
}
