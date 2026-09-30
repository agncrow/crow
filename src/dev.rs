//! `crow dev`: the program served by the interpreter and reloaded as its
//! files change (ui-framework.md, Phase 5). A change is checked first, by
//! the gate `crow run -i` passes; a program that passes replaces the one
//! serving, and every tab reconnects and resumes its session — the dev
//! runtime keeps sessions, as `serve[…, keep = true]` does — drawn by the
//! new code. A change that breaks the program leaves the last good one
//! serving, and its pages show the refusal and where it is until a change
//! mends it. A change to what a page's shell carries (css/, widgets/)
//! reloads the tabs. A script is run again.

use std::collections::BTreeMap;
use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

/// The files `crow dev` watches: the program's code, its migrations and its
/// crowfile, which restart it, and its page shell's styles and widgets,
/// which reload the tabs.
fn watched(root: &Path) -> BTreeMap<PathBuf, SystemTime> {
    fn walk(dir: &Path, out: &mut BTreeMap<PathBuf, SystemTime>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for e in entries.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            if p.is_dir() {
                if !name.starts_with('.') && !matches!(name.as_str(), "target" | "node_modules" | "crow_modules") {
                    walk(&p, out);
                }
                continue;
            }
            let ext = p.extension().map(|x| x.to_string_lossy().to_string()).unwrap_or_default();
            if name == "crowfile" || matches!(ext.as_str(), "co" | "crow" | "sql" | "css" | "js") {
                if let Ok(t) = e.metadata().and_then(|m| m.modified()) {
                    out.insert(p, t);
                }
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, &mut out);
    out
}

/// Whether a changed file is the program's (a restart), not its shell's (a
/// reload of the tabs).
fn is_code(path: &Path) -> bool {
    let ext = path.extension().map(|x| x.to_string_lossy().to_string()).unwrap_or_default();
    path.file_name().is_some_and(|n| n == "crowfile") || matches!(ext.as_str(), "co" | "crow" | "sql")
}

/// The program checked as `crow run -i` would take it: none, or the refusal.
fn check(exe: &Path, entry: &str) -> Result<(), String> {
    let out = Command::new(exe).args(["build", "--ir", entry]).stdout(Stdio::null()).stderr(Stdio::piped()).output().map_err(|e| format!("cannot check the program: {}", e))?;
    if out.status.success() {
        return Ok(());
    }
    // the entry's refusals name its line, a part's its file too
    let said = String::from_utf8_lossy(&out.stderr).trim().to_string();
    Err(said.lines().map(|l| if l.starts_with("line ") { format!("{}: {}", entry, l) } else { l.to_string() }).collect::<Vec<_>>().join("\n"))
}

/// The interpreter serving the program, its output passed on, the port it
/// says it listens on noted.
fn start(exe: &Path, file: Option<&str>, token: &str, port: &Arc<Mutex<Option<u16>>>) -> Option<Child> {
    let mut args = vec!["run".to_string(), "-i".to_string()];
    args.extend(file.map(str::to_string));
    let mut child = match Command::new(exe).args(&args).env("CROW_DEV", token).stdin(Stdio::inherit()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("crow dev: cannot start the program: {}", e);
            return None;
        }
    };
    *port.lock().unwrap_or_else(|p| p.into_inner()) = None;
    let pass = |from: Box<dyn Read + Send>, to_err: bool, port: Arc<Mutex<Option<u16>>>| {
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(from).lines().map_while(Result::ok) {
                if let Some(at) = line.strip_prefix("crow server listening on http://") {
                    if let Some(p) = at.rsplit(':').next().and_then(|p| p.trim().parse::<u16>().ok()) {
                        *port.lock().unwrap_or_else(|p| p.into_inner()) = Some(p);
                    }
                }
                if to_err { eprintln!("{}", line) } else { println!("{}", line) }
            }
        });
    };
    if let Some(out) = child.stdout.take() {
        pass(Box::new(out), false, port.clone());
    }
    if let Some(err) = child.stderr.take() {
        pass(Box::new(err), true, port.clone());
    }
    Some(child)
}

/// The program serving stopped as a person stops it — its queries in
/// flight stopped with it — and its port free before the next takes it.
fn stop(child: Option<Child>, port: &Arc<Mutex<Option<u16>>>) {
    let Some(mut child) = child else { return };
    let held = *port.lock().unwrap_or_else(|p| p.into_inner());
    let _ = Command::new("kill").arg("-TERM").arg(child.id().to_string()).output();
    let by = Instant::now() + Duration::from_secs(5);
    while Instant::now() < by {
        if let Ok(Some(_)) = child.try_wait() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = child.kill();
    let _ = child.wait();
    if let Some(p) = held {
        let by = Instant::now() + Duration::from_secs(5);
        while std::net::TcpStream::connect(("127.0.0.1", p)).is_ok() && Instant::now() < by {
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

/// A word to the program serving, for its pages: the refusal to show, or
/// a reload of the tabs. Only the program `crow dev` started takes it.
fn tell(port: u16, token: &str, path: &str, body: &str) {
    if let Ok(mut s) = std::net::TcpStream::connect(("127.0.0.1", port)) {
        let _ = write!(
            s,
            "POST {} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nX-Crow-Dev: {}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            path,
            port,
            token,
            body.len(),
            body
        );
        let mut answer = String::new();
        let _ = s.read_to_string(&mut answer);
    }
}

/// `crow dev [file.co]`.
pub fn cmd_dev(file: Option<&str>) {
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => {
            eprintln!("crow dev: cannot find the crow binary: {}", e);
            std::process::exit(1);
        }
    };
    let entry = file.map(str::to_string).unwrap_or_else(|| crate::parse_crowfile().entry);
    let root = crate::project_root(Path::new(&entry).parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new(".")));
    let token = crow_runtime::signin::random_hex(16).unwrap_or_else(|_| format!("{}", std::process::id()));
    let port: Arc<Mutex<Option<u16>>> = Arc::new(Mutex::new(None));
    println!("crow dev: {} — each change reloads it; Ctrl-C stops", entry);
    let mut seen = watched(&root);
    let mut child = match check(&exe, &entry) {
        Ok(()) => start(&exe, file, &token, &port),
        Err(e) => {
            eprintln!("{}\ncrow dev: waiting for a change that mends it", e);
            None
        }
    };
    loop {
        std::thread::sleep(Duration::from_millis(250));
        let mut now = watched(&root);
        if now == seen {
            continue;
        }
        // an editor writes a file in steps: the change is over when a
        // look finds nothing new
        loop {
            std::thread::sleep(Duration::from_millis(120));
            let again = watched(&root);
            if again == now {
                break;
            }
            now = again;
        }
        let changed: Vec<PathBuf> = now.iter().filter(|(p, t)| seen.get(*p) != Some(t)).map(|(p, _)| p.clone()).chain(seen.keys().filter(|p| !now.contains_key(*p)).cloned()).collect();
        seen = now;
        let shown: Vec<String> = changed.iter().map(|p| p.strip_prefix(&root).unwrap_or(p).display().to_string()).collect();
        let held = *port.lock().unwrap_or_else(|p| p.into_inner());
        if changed.iter().any(|p| is_code(p)) {
            let began = Instant::now();
            match check(&exe, &entry) {
                Ok(()) => {
                    stop(child.take(), &port);
                    child = start(&exe, file, &token, &port);
                    println!("crow dev: {} changed — reloaded in {:.1}s", shown.join(", "), began.elapsed().as_secs_f64());
                }
                Err(e) => {
                    eprintln!("crow dev: {} changed — refused, the last good program keeps serving\n{}", shown.join(", "), e);
                    if let Some(p) = held {
                        tell(p, &token, "/_dev/error", &e);
                    }
                }
            }
        } else if let Some(p) = held {
            println!("crow dev: {} changed — the tabs reload", shown.join(", "));
            tell(p, &token, "/_dev/reload", "");
        }
    }
}
