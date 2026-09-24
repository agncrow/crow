//! `crow test --served`: every page in `tests/served/` — and every
//! reference app, `examples/<app>/main.co` with a `main.events` beside it,
//! served from a fresh copy — is served by the compiled program and by the
//! interpreter, driven with the same events, and the pages the two settle
//! on after each step must agree (session-layer-design.md §3.11). `name.events` beside `name.co` is the
//! script: a JSON list of steps — `{"open": "/?day=2026-09-04"}` first to
//! load the page from an address, `{"event": "inc"}` or `{"event":
//! "__field:name", "value": "Ada"}` to act, `{"ask": "end_session"}` to
//! press the button that asks before `end_session`, and `{"expect": "…"}`
//! or `{"absent": "…"}` to say what the canonical page must, or must not,
//! hold at that point — so a page both modes render alike is also the
//! page it should be. A later `{"open": "/"}` opens the page again, with
//! the cookie the page was given — a sign-in's, when the page traded a
//! ticket for one, as a browser would. The runner fetches every file the
//! page is handed, and `{"downloaded": "name.csv", "holds": "…"}` says one
//! came, holding that text. `{"upload": "__field:upload", "file":
//! "fixtures/trades.csv"}` picks that file for the page's file field, and
//! the runner sends it the way the page's script does.

use std::io::{Read, Write};
use std::time::{Duration, Instant};

use crow_web::view::CrowVNode;

/// A page's socket, as the runner drives it.
type Socket = tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<std::net::TcpStream>>;

/// The port a program serves on: its `serve[NNNN`.
fn port_of(source: &str) -> Option<u16> {
    let at = source.find("serve[")? + "serve[".len();
    source[at..].chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().ok()
}

fn request(port: u16, method: &str, path: &str, cookie: &Option<String>) -> Result<String, String> {
    let mut http = std::net::TcpStream::connect(("127.0.0.1", port)).map_err(|e| e.to_string())?;
    let cookie_line = cookie.as_ref().map(|c| format!("Cookie: crow_reader={}\r\n", c)).unwrap_or_default();
    write!(
        http,
        "{} {} HTTP/1.1\r\nHost: 127.0.0.1:{p}\r\nOrigin: http://127.0.0.1:{p}\r\n{}Content-Length: 0\r\nConnection: close\r\n\r\n",
        method, path, cookie_line, p = port
    )
    .map_err(|e| e.to_string())?;
    let mut page = String::new();
    http.read_to_string(&mut page).map_err(|e| e.to_string())?;
    Ok(page)
}

/// POST a file's bytes, as the page's script sends a picked file.
fn request_with_body(port: u16, path: &str, body: &[u8]) -> Result<String, String> {
    let mut http = std::net::TcpStream::connect(("127.0.0.1", port)).map_err(|e| e.to_string())?;
    write!(
        http,
        "POST {} HTTP/1.1\r\nHost: 127.0.0.1:{p}\r\nOrigin: http://127.0.0.1:{p}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        path, body.len(), p = port
    )
    .map_err(|e| e.to_string())?;
    http.write_all(body).map_err(|e| e.to_string())?;
    let mut answer = String::new();
    http.read_to_string(&mut answer).map_err(|e| e.to_string())?;
    Ok(answer)
}

/// Trade a sign-in's ticket for its cookie, as the page's script does: the
/// jar holds the sign-in's cookie, or nothing after a sign-out.
fn trade_ticket(port: u16, url: &str, jar: &mut Option<String>) -> Result<(), String> {
    let answer = request(port, "POST", url, &None)?;
    let set = answer
        .lines()
        .find(|l| l.to_ascii_lowercase().starts_with("set-cookie:"))
        .ok_or_else(|| format!("the ticket bought no cookie: {}", answer.lines().next().unwrap_or("")))?;
    let value = set.split("crow_reader=").nth(1).unwrap_or("").split(';').next().unwrap_or("").to_string();
    *jar = if value.is_empty() { None } else { Some(value) };
    Ok(())
}

/// The page opened at `path` with the jar's cookie, and its socket.
fn open_page(port: u16, path: &str, jar: &Option<String>) -> Result<(CrowVNode, Socket), String> {
    use tungstenite::client::IntoClientRequest;
    let first = request(port, "GET", path, jar)?;
    let token: String = first.split("pageToken = '").nth(1).unwrap_or("").chars().take_while(|c| c.is_ascii_hexdigit()).collect();
    let page = CrowVNode::raw(
        &first
            .split("<div id=\"crow-root\">")
            .nth(1)
            .and_then(|rest| rest.rsplit_once("</div>\n<script>").map(|(inner, _)| inner.to_string()))
            .unwrap_or_default(),
    )
    .as_built();
    let mut req = format!("ws://127.0.0.1:{}/_ws{}?p={}", port, path.split('?').next().unwrap_or("/"), token)
        .into_client_request()
        .map_err(|e| e.to_string())?;
    if let Some(c) = jar {
        req.headers_mut().insert("Cookie", format!("crow_reader={}", c).parse().map_err(|_| "a cookie that is no header")?);
    }
    let (mut ws, _) = tungstenite::connect(req).map_err(|e| format!("the socket did not open: {}", e))?;
    if let tungstenite::stream::MaybeTlsStream::Plain(s) = ws.get_mut() {
        let _ = s.set_read_timeout(Some(Duration::from_millis(100)));
    }
    Ok((page, ws))
}

/// What the page shows, in one canonical form.
fn canonical(html: &str) -> String {
    crate::harness::harness_canonical(&crate::harness::harness_parse_html(html))
}

/// Read what the server sends until it has been quiet for a moment; the
/// page is what the renders made of it — each a whole page, or the changes
/// a browser applies to the one it has. A change that does not fit fails
/// the run: the server's idea of the page and the page's own have parted.
fn settle(ws: &mut Socket, page: &mut CrowVNode, port: u16, jar: &mut Option<String>, files: &mut Vec<(String, String)>, picked: &mut Option<Vec<u8>>) -> Result<(), String> {
    let end = Instant::now() + Duration::from_secs(20);
    let mut quiet_since = Instant::now();
    while Instant::now() < end && quiet_since.elapsed() < Duration::from_millis(500) {
        match ws.read() {
            Ok(tungstenite::Message::Text(t)) => {
                if let Ok(m) = serde_json::from_str::<serde_json::Value>(&t) {
                    if let Some(h) = m["html"].as_str() {
                        *page = CrowVNode::raw(h).as_built();
                    } else if m["type"] == "patches" {
                        let changes = m["patches"].as_array().cloned().unwrap_or_default();
                        page.apply(&changes).map_err(|e| format!("a change did not fit the page: {}", e))?;
                    } else if m["type"] == "sign" {
                        trade_ticket(port, m["url"].as_str().unwrap_or(""), jar)?;
                    } else if m["type"] == "upload" {
                        // the ticket for the file picked: send it by it
                        let bytes = picked.take().ok_or("an upload ticket came for no file picked")?;
                        let answer = request_with_body(port, m["url"].as_str().unwrap_or(""), &bytes)?;
                        if !answer.starts_with("HTTP/1.1 204") {
                            return Err(format!("the upload was refused: {}", answer.lines().next().unwrap_or("")));
                        }
                    } else if m["type"] == "download" {
                        let answer = request(port, "GET", m["url"].as_str().unwrap_or(""), jar)?;
                        let body = answer.split_once("\r\n\r\n").map(|(_, b)| b.to_string()).unwrap_or_default();
                        files.push((m["name"].as_str().unwrap_or("").to_string(), body));
                    }
                }
                quiet_since = Instant::now();
            }
            Ok(_) => quiet_since = Instant::now(),
            Err(tungstenite::Error::Io(e)) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
            Err(_) => break,
        }
    }
    Ok(())
}

/// The question a button asks before `handler`, as the page offers it.
fn question_for(page: &str, handler: &str) -> Option<String> {
    let marker = format!("[\"{}\"", handler);
    for piece in page.split("data-crow-args=\"").skip(1) {
        let raw = piece.split('"').next().unwrap_or("");
        let args = raw.replace("&quot;", "\"").replace("&#39;", "'").replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&");
        if args.starts_with(&marker) {
            return Some(args);
        }
    }
    None
}

/// Serve `file` in one mode, drive it with `steps`, and return the page
/// after each step.
fn drive(exe: &std::path::Path, mode: &[&str], file: &std::path::Path, port: u16, steps: &[serde_json::Value]) -> Result<Vec<String>, String> {
    use std::os::unix::process::CommandExt;
    let dir = file.parent().unwrap_or(std::path::Path::new("."));
    // its own process group: `crow run` starts the built program as a child,
    // and stopping the group stops both
    let mut child = std::process::Command::new(exe)
        .process_group(0)
        .args(mode)
        .arg(file.file_name().unwrap_or_default())
        .current_dir(dir)
        .env("CROW_BUILD_PROFILE", "dev")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot start: {}", e))?;
    let result = (|| {
        // a compiled program builds first
        let up_by = Instant::now() + Duration::from_secs(600);
        while std::net::TcpStream::connect(("127.0.0.1", port)).is_err() {
            if Instant::now() > up_by {
                return Err("it never started serving".to_string());
            }
            if let Ok(Some(status)) = child.try_wait() {
                let mut err = String::new();
                if let Some(mut e) = child.stderr.take() {
                    let _ = e.read_to_string(&mut err);
                }
                return Err(format!("it exited ({}) before serving: {}", status, err.lines().last().unwrap_or("")));
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        let open = steps.first().and_then(|s| s["open"].as_str()).unwrap_or("/").to_string();
        let mut jar: Option<String> = None;
        let mut files: Vec<(String, String)> = Vec::new();
        let mut picked: Option<Vec<u8>> = None;
        let (mut page, mut ws) = open_page(port, &open, &jar)?;
        settle(&mut ws, &mut page, port, &mut jar, &mut files, &mut picked)?;
        let mut pages = vec![canonical(&page.to_html())];
        let mut seq = 0;
        for (i, step) in steps.iter().enumerate() {
            // the first step's address opened the page above; a later one
            // opens it again, with the cookie the page was given
            if let Some(path) = step["open"].as_str() {
                if i == 0 {
                    continue;
                }
                let _ = ws.close(None);
                let (p, w) = open_page(port, path, &jar)?;
                page = p;
                ws = w;
                settle(&mut ws, &mut page, port, &mut jar, &mut files, &mut picked)?;
                pages.push(canonical(&page.to_html()));
                continue;
            }
            let now = pages.last().cloned().unwrap_or_default();
            if let Some(name) = step["downloaded"].as_str() {
                let holds = step["holds"].as_str().unwrap_or("");
                match files.iter().rev().find(|(n, _)| n == name) {
                    Some((_, body)) if body.contains(holds) => {}
                    Some((_, body)) => return Err(format!("the download {} does not hold {:?}: {:?}", name, holds, body)),
                    None => return Err(format!("no download named {} came", name)),
                }
                continue;
            }
            if let Some(want) = step["expect"].as_str() {
                if !now.contains(want) {
                    return Err(format!("the page does not hold {:?}: {}", want, now));
                }
                continue;
            }
            if let Some(unwanted) = step["absent"].as_str() {
                if now.contains(unwanted) {
                    return Err(format!("the page holds {:?}: {}", unwanted, now));
                }
                continue;
            }
            seq += 1;
            let msg = if let Some(field) = step["upload"].as_str() {
                // a file picked for the field: the page asks for a ticket first
                let rel = step["file"].as_str().unwrap_or("");
                let bytes = std::fs::read(dir.join(rel)).map_err(|e| format!("the fixture {}: {}", rel, e))?;
                let name = std::path::Path::new(rel).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                let size = bytes.len();
                picked = Some(bytes);
                serde_json::json!({ "event": "__upload", "value": serde_json::json!([field, name, "text/csv", size]).to_string(), "seq": seq })
            } else if let Some(h) = step["ask"].as_str() {
                let args = question_for(&page.to_html(), h).ok_or_else(|| format!("the page asks nothing before '{}'", h))?;
                serde_json::json!({ "event": "__ask", "value": args, "seq": seq })
            } else {
                let mut m = serde_json::json!({ "event": step["event"], "seq": seq });
                if let Some(v) = step.get("value") {
                    m["value"] = v.clone();
                }
                m
            };
            ws.send(tungstenite::Message::Text(msg.to_string().into())).map_err(|e| e.to_string())?;
            settle(&mut ws, &mut page, port, &mut jar, &mut files, &mut picked)?;
            pages.push(canonical(&page.to_html()));
        }
        let _ = ws.close(None);
        Ok(pages)
    })();
    // `crow run` alone is stopped, as a person stops it: the program it
    // runs stops with it (a signal to crow run is handed on), and the port
    // is free before the next mode takes it
    let _ = std::process::Command::new("kill").arg("-TERM").arg(child.id().to_string()).output();
    let _ = child.wait();
    let free_by = Instant::now() + Duration::from_secs(10);
    while std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() && Instant::now() < free_by {
        std::thread::sleep(Duration::from_millis(100));
    }
    let kept_serving = std::net::TcpStream::connect(("127.0.0.1", port)).is_ok();
    // whatever is left of the group goes, so the next mode can serve
    let _ = std::process::Command::new("kill").arg("-KILL").arg("--").arg(format!("-{}", child.id())).output();
    if kept_serving && result.is_ok() {
        return Err("stopping `crow run` left its program serving".to_string());
    }
    result
}

/// The reference apps: `examples/<app>/main.co` with a `main.events` beside it.
fn reference_apps() -> Vec<std::path::PathBuf> {
    let mut apps: Vec<std::path::PathBuf> = std::fs::read_dir("examples")
        .map(|rd| rd.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.join("main.events").is_file() && p.join("main.co").is_file()).collect())
        .unwrap_or_default();
    apps.sort();
    apps
}

/// A fresh copy of an app to serve, under `target/crow_served/<app>`: its
/// sources as version control sees them (tracked, or new and not ignored),
/// so its database, its files and its keys start empty in each mode. The
/// copy's build cache is kept between runs.
fn fresh_copy(app: &std::path::Path) -> Result<std::path::PathBuf, String> {
    let name = app.file_name().ok_or("an app with no name")?;
    let to = std::path::Path::new("target").join("crow_served").join(name);
    std::fs::create_dir_all(&to).map_err(|e| format!("cannot create {}: {}", to.display(), e))?;
    for entry in std::fs::read_dir(&to).map_err(|e| e.to_string())?.flatten() {
        if entry.file_name() == ".crow_build" {
            continue;
        }
        let p = entry.path();
        let _ = if p.is_dir() { std::fs::remove_dir_all(&p) } else { std::fs::remove_file(&p) };
    }
    let listed = std::process::Command::new("git")
        .args(["ls-files", "-z", "--cached", "--others", "--exclude-standard", "--"])
        .arg(app)
        .output()
        .map_err(|e| format!("cannot list {} with git: {}", app.display(), e))?;
    if !listed.status.success() {
        return Err(format!("git ls-files {}: {}", app.display(), String::from_utf8_lossy(&listed.stderr).trim()));
    }
    // NUL-separated, so a name git would quote comes through as it is
    for line in String::from_utf8_lossy(&listed.stdout).split('\0').filter(|l| !l.is_empty()) {
        let from = std::path::Path::new(line);
        let Ok(rel) = from.strip_prefix(app) else { continue };
        if !from.is_file() {
            continue;
        }
        let dest = to.join(rel);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::copy(from, &dest).map_err(|e| format!("cannot copy {}: {}", from.display(), e))?;
    }
    Ok(to.join("main.co"))
}

/// Run the corpus; the process's exit status.
pub fn run() -> i32 {
    let dir = std::path::Path::new("tests/served");
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
        .map(|rd| rd.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().map_or(false, |x| x == "co")).collect())
        .unwrap_or_default();
    files.sort();
    let apps = reference_apps();
    files.extend(apps.iter().map(|a| a.join("main.co")));
    if files.is_empty() {
        eprintln!("error: no tests/served/*.co found (run from the project root)");
        return 1;
    }
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => {
            eprintln!("error locating crow: {}", e);
            return 1;
        }
    };
    let start = Instant::now();
    let mut failures = 0;
    println!("served: {} pages, compiled vs interpreter\n", files.len());
    for file in &files {
        let app = file.parent().filter(|d| apps.iter().any(|a| a == d));
        let name = match app {
            Some(a) => a.display().to_string(),
            None => file.file_name().unwrap_or_default().to_string_lossy().to_string(),
        };
        let source = std::fs::read_to_string(file).unwrap_or_default();
        let Some(port) = port_of(&source) else {
            println!("  FAIL  {}: no serve[port]", name);
            failures += 1;
            continue;
        };
        let steps: Vec<serde_json::Value> = std::fs::read_to_string(file.with_extension("events"))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        // an app is served from a fresh copy in each mode
        let serve = |mode: &[&str]| match app {
            Some(a) => fresh_copy(a).and_then(|copy| drive(&exe, mode, &copy, port, &steps)),
            None => drive(&exe, mode, file, port, &steps),
        };
        let compiled = serve(&["run"]);
        let interpreted = serve(&["run", "-i"]);
        match (compiled, interpreted) {
            (Ok(c), Ok(i)) if c == i => println!("  ok    {} ({} steps)", name, c.len() - 1),
            (Ok(c), Ok(i)) => {
                failures += 1;
                let n = c.iter().zip(i.iter()).position(|(a, b)| a != b).unwrap_or(c.len().min(i.len()));
                println!("  FAIL  {}: the pages differ after step {}", name, n);
                println!("        compiled:    {}", c.get(n).map(|s| s.as_str()).unwrap_or("<none>"));
                println!("        interpreter: {}", i.get(n).map(|s| s.as_str()).unwrap_or("<none>"));
            }
            (c, i) => {
                failures += 1;
                println!("  FAIL  {}", name);
                if let Err(e) = c {
                    println!("        compiled: {}", e);
                }
                if let Err(e) = i {
                    println!("        interpreter: {}", e);
                }
            }
        }
    }
    println!();
    if failures == 0 {
        println!("✓ {} pages serve the same in both modes ({:.1}s)", files.len(), start.elapsed().as_secs_f64());
        0
    } else {
        println!("✗ {} of {} pages differ between modes ({:.1}s)", failures, files.len(), start.elapsed().as_secs_f64());
        1
    }
}
