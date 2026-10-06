//! `crow kernel`: Crow in Jupyter (spec, Notebooks). Jupyter starts the
//! kernel with a connection file naming five sockets (the Jupyter
//! messaging protocol, version 5.3, over ZeroMQ): each cell is fed to one
//! interpreter session — the REPL's, so a cell sees what the cells before
//! it defined — what it logs is streamed back as it runs, and its last
//! expression's value is the cell's result, a frame as a table.
//! `crow kernel install` writes the kernel spec Jupyter finds it by.

use serde_json::{json, Value};

/// The connection file Jupyter writes: where each socket listens, and the
/// key every message is signed with.
#[derive(serde::Deserialize, Debug, Clone)]
pub struct Connection {
    pub transport: String,
    pub ip: String,
    pub shell_port: u16,
    pub iopub_port: u16,
    pub stdin_port: u16,
    pub control_port: u16,
    pub hb_port: u16,
    pub key: String,
    pub signature_scheme: String,
}

impl Connection {
    pub fn endpoint(&self, port: u16) -> String {
        format!("{}://{}:{}", self.transport, self.ip, port)
    }
}

/// A message's four JSON parts, as the wire carries them after the
/// delimiter and the signature.
#[derive(Debug, Clone)]
pub struct Message {
    pub identities: Vec<Vec<u8>>,
    pub header: Value,
    pub parent: Value,
    pub metadata: Value,
    pub content: Value,
}

const DELIMITER: &[u8] = b"<IDS|MSG>";

/// The HMAC of a message's parts, in hex, as the protocol signs it (an
/// empty key signs nothing).
pub fn signature(key: &str, parts: &[&[u8]]) -> String {
    use hmac::{Hmac, Mac};
    if key.is_empty() {
        return String::new();
    }
    let Ok(mut mac) = <Hmac<sha2::Sha256> as Mac>::new_from_slice(key.as_bytes()) else {
        return String::new();
    };
    for p in parts {
        mac.update(p);
    }
    mac.finalize().into_bytes().iter().map(|b| format!("{:02x}", b)).collect()
}

impl Message {
    /// A message read off the wire: the identities before the delimiter,
    /// the signature checked, the four parts parsed.
    pub fn parse(frames: &[Vec<u8>], key: &str) -> Result<Message, String> {
        let at = frames.iter().position(|f| f.as_slice() == DELIMITER).ok_or("no delimiter")?;
        let rest = &frames[at + 1..];
        if rest.len() < 5 {
            return Err("a message has a signature and four parts".into());
        }
        let expected = signature(key, &[&rest[1], &rest[2], &rest[3], &rest[4]]);
        if !key.is_empty() && expected.as_bytes() != rest[0].as_slice() {
            return Err("a message's signature does not match".into());
        }
        let part = |b: &[u8]| serde_json::from_slice::<Value>(b).map_err(|e| e.to_string());
        Ok(Message { identities: frames[..at].to_vec(), header: part(&rest[1])?, parent: part(&rest[2])?, metadata: part(&rest[3])?, content: part(&rest[4])? })
    }

    /// The frames that carry a message, signed.
    pub fn frames(&self, key: &str) -> Vec<Vec<u8>> {
        let (h, p, m, c) = (self.header.to_string(), self.parent.to_string(), self.metadata.to_string(), self.content.to_string());
        let sig = signature(key, &[h.as_bytes(), p.as_bytes(), m.as_bytes(), c.as_bytes()]);
        let mut out = self.identities.clone();
        out.push(DELIMITER.to_vec());
        out.push(sig.into_bytes());
        out.extend([h.into_bytes(), p.into_bytes(), m.into_bytes(), c.into_bytes()]);
        out
    }

    pub fn msg_type(&self) -> &str {
        self.header.get("msg_type").and_then(Value::as_str).unwrap_or("")
    }
}

/// A reply or a broadcast to `parent`: a fresh header of `msg_type` in the
/// parent's session.
pub fn reply(parent: &Message, msg_type: &str, content: Value, identities: Vec<Vec<u8>>) -> Message {
    let session = parent.header.get("session").cloned().unwrap_or(json!(""));
    Message {
        identities,
        header: json!({
            "msg_id": fresh_id(),
            "session": session,
            "username": "crow",
            "date": now_iso(),
            "msg_type": msg_type,
            "version": "5.3",
        }),
        parent: parent.header.clone(),
        metadata: json!({}),
        content,
    }
}

fn fresh_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("{:x}-{:x}-{}", nanos, std::process::id(), N.fetch_add(1, Ordering::Relaxed))
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, true)
}

/// What `kernel_info_request` is answered.
pub fn kernel_info() -> Value {
    json!({
        "status": "ok",
        "protocol_version": "5.3",
        "implementation": "crow",
        "implementation_version": env!("CARGO_PKG_VERSION"),
        "language_info": {
            "name": "crow",
            "version": env!("CARGO_PKG_VERSION"),
            "mimetype": "text/x-crow",
            "file_extension": ".co",
            "pygments_lexer": "text",
            "codemirror_mode": "text",
        },
        "banner": format!("Crow {}", env!("CARGO_PKG_VERSION")),
        "help_links": [],
    })
}

/// The kernel spec `crow kernel install` writes: how Jupyter starts the
/// kernel, and what it calls it.
pub fn kernel_spec(crow: &str) -> Value {
    json!({
        "argv": [crow, "kernel", "{connection_file}"],
        "display_name": "Crow",
        "language": "crow",
        "interrupt_mode": "message",
    })
}

/// Where Jupyter looks for a user's kernels: `jupyter --data-dir`'s
/// `kernels/crow`, or the platform's default when Jupyter is not on the
/// path.
pub fn kernel_dir() -> std::path::PathBuf {
    let data = std::process::Command::new("jupyter")
        .arg("--data-dir")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .map(std::path::PathBuf::from);
    let home = std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default());
    let data = data.unwrap_or_else(|| if cfg!(target_os = "macos") { home.join("Library/Jupyter") } else { home.join(".local/share/jupyter") });
    data.join("kernels").join("crow")
}

/// What a cell prints, while it runs: the process's stdout and stderr
/// point at pipes whose readers hand each piece of text to `each` as it
/// comes (`log` in a loop streams, it does not wait for the cell), and both
/// are put back when the cell ends.
pub struct Capture {
    saved: Vec<(i32, i32)>,
    readers: Vec<std::thread::JoinHandle<()>>,
}

#[cfg(unix)]
impl Capture {
    pub fn start(each: std::sync::Arc<dyn Fn(&'static str, String) + Send + Sync>) -> Result<Capture, String> {
        use std::io::Write;
        use std::os::fd::FromRawFd;
        let _ = std::io::stdout().flush();
        let _ = std::io::stderr().flush();
        let mut saved = Vec::new();
        let mut readers = Vec::new();
        for (fd, name) in [(1, "stdout"), (2, "stderr")] {
            let mut ends = [0i32; 2];
            // SAFETY: plain descriptor calls on descriptors this function
            // owns; the saved copy puts the original back in `finish`
            let (read_end, copy) = unsafe {
                if libc::pipe(ends.as_mut_ptr()) != 0 {
                    return Err(format!("kernel: cannot capture {}: {}", name, std::io::Error::last_os_error()));
                }
                let copy = libc::dup(fd);
                libc::dup2(ends[1], fd);
                libc::close(ends[1]);
                (std::fs::File::from_raw_fd(ends[0]), copy)
            };
            saved.push((fd, copy));
            let each = each.clone();
            readers.push(std::thread::spawn(move || read_pieces(read_end, name, &*each)));
        }
        Ok(Capture { saved, readers })
    }

    /// The cell has ended: what it printed is flushed and read to its end,
    /// and stdout and stderr are the process's own again.
    pub fn finish(self) {
        use std::io::Write;
        let _ = std::io::stdout().flush();
        let _ = std::io::stderr().flush();
        for (fd, copy) in &self.saved {
            // SAFETY: puts back the descriptor `start` saved; the pipe's
            // write end closes with it, so its reader ends
            unsafe {
                libc::dup2(*copy, *fd);
                libc::close(*copy);
            }
        }
        for r in self.readers {
            let _ = r.join();
        }
    }
}

/// Elsewhere a cell's output goes where the process's does.
#[cfg(not(unix))]
impl Capture {
    pub fn start(_each: std::sync::Arc<dyn Fn(&'static str, String) + Send + Sync>) -> Result<Capture, String> {
        Ok(Capture { saved: Vec::new(), readers: Vec::new() })
    }

    pub fn finish(self) {}
}

/// A pipe read to its end, each piece handed on as text; a character split
/// between two reads is handed on whole.
#[cfg(unix)]
fn read_pieces(mut from: std::fs::File, name: &'static str, each: &(dyn Fn(&'static str, String) + Send + Sync)) {
    use std::io::Read;
    let mut buf = [0u8; 8192];
    let mut carry: Vec<u8> = Vec::new();
    loop {
        let n = match from.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        carry.extend_from_slice(&buf[..n]);
        let upto = match std::str::from_utf8(&carry) {
            Ok(_) => carry.len(),
            Err(e) if e.error_len().is_none() => e.valid_up_to(),
            Err(_) => carry.len(),
        };
        if upto > 0 {
            let rest = carry.split_off(upto);
            each(name, String::from_utf8_lossy(&carry).into_owned());
            carry = rest;
        }
    }
    if !carry.is_empty() {
        each(name, String::from_utf8_lossy(&carry).into_owned());
    }
}

// ── the kernel ──────────────────────────────────────────────────────

/// A cell to run: its code, the request it answers, and where its reply
/// goes.
struct Job {
    code: String,
    request: Message,
    silent: bool,
    done: tokio::sync::oneshot::Sender<Value>,
}

/// A frame as the notebook shows it: a table of its first rows, the count
/// of the rest under it.
fn frame_html(f: &crow_runtime::frame::CrowFrame) -> String {
    const SHOWN: usize = 50;
    let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    let mut out = String::from("<table><thead><tr>");
    for c in &f.columns {
        out.push_str(&format!("<th>{}</th>", esc(c)));
    }
    out.push_str("</tr></thead><tbody>");
    for r in 0..f.nrows().min(SHOWN) {
        out.push_str("<tr>");
        for i in 0..f.ncols() {
            let cell = f.data[i].fmt_cell(r, f.validity[i][r]);
            out.push_str(&format!("<td>{}</td>", esc(&cell)));
        }
        out.push_str("</tr>");
    }
    out.push_str("</tbody></table>");
    if f.nrows() > SHOWN {
        out.push_str(&format!("<p>… {} more rows ({} in all)</p>", f.nrows() - SHOWN, f.nrows()));
    }
    out
}

/// The cells, run one at a time on the thread that holds the session: a
/// cell sees what the ones before it defined. What it prints streams to
/// iopub as it comes; its value is its result; the reply's content goes
/// back to the shell.
fn run_cells(jobs: std::sync::mpsc::Receiver<Job>, iopub: tokio::sync::mpsc::UnboundedSender<Message>, cancel: std::sync::Arc<std::sync::atomic::AtomicBool>) {
    let mut session = crate::ir_interp::Session::new();
    session.cancel_with(cancel.clone());
    let mut count: u64 = 0;
    for job in jobs {
        cancel.store(false, std::sync::atomic::Ordering::SeqCst);
        if !job.silent {
            count += 1;
            let _ = iopub.send(reply(&job.request, "execute_input", json!({"code": job.code, "execution_count": count}), vec![b"execute_input".to_vec()]));
        }
        let streams = iopub.clone();
        let parent = job.request.clone();
        let each: std::sync::Arc<dyn Fn(&'static str, String) + Send + Sync> = std::sync::Arc::new(move |name, text| {
            let _ = streams.send(reply(&parent, "stream", json!({"name": name, "text": text}), vec![b"stream".to_vec()]));
        });
        let capture = Capture::start(each);
        let outcome = crate::build(&job.code).and_then(|stmts| session.feed(&stmts));
        if let Ok(c) = capture {
            c.finish();
        }
        let content = match outcome {
            Ok(value) => {
                if let Some(v) = value.filter(|v| !matches!(v.value, crate::value::Value::Done))
                    && !job.silent
                {
                    let mut data = json!({"text/plain": format!("{}", v)});
                    if let crate::value::Value::Frame(f) = &v.value {
                        data["text/html"] = json!(frame_html(f));
                    }
                    let _ = iopub.send(reply(&job.request, "execute_result", json!({"execution_count": count, "data": data, "metadata": {}}), vec![b"execute_result".to_vec()]));
                }
                json!({"status": "ok", "execution_count": count, "user_expressions": {}, "payload": []})
            }
            Err(e) => {
                // an interrupt stops the cell where its loop turned (`line 3:
                // execution cancelled`)
                let (ename, evalue) = if e.ends_with("execution cancelled") {
                    ("Interrupted", e.replace("execution cancelled", "the cell was interrupted"))
                } else {
                    ("CrowError", e.clone())
                };
                let _ = iopub.send(reply(&job.request, "error", json!({"ename": ename, "evalue": evalue, "traceback": [evalue]}), vec![b"error".to_vec()]));
                json!({"status": "error", "execution_count": count, "ename": ename, "evalue": evalue, "traceback": [evalue]})
            }
        };
        let _ = job.done.send(content);
    }
}

fn to_zmq(frames: Vec<Vec<u8>>) -> Option<zeromq::ZmqMessage> {
    zeromq::ZmqMessage::try_from(frames.into_iter().map(bytes::Bytes::from).collect::<Vec<_>>()).ok()
}

fn from_zmq(m: zeromq::ZmqMessage) -> Vec<Vec<u8>> {
    m.into_vec().into_iter().map(|b| b.to_vec()).collect()
}

/// A status broadcast: busy while a request is handled, idle after.
fn status(parent: &Message, state: &str) -> Message {
    reply(parent, "status", json!({"execution_state": state}), vec![b"status".to_vec()])
}

/// Whether a console's line is a whole cell: code that builds is, a block
/// opened and not yet given a body is not (the console indents the next
/// line), anything else is the error it is.
fn is_complete(code: &str) -> Value {
    if crate::build(code).is_ok() {
        return json!({"status": "complete"});
    }
    let last = code.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim_start();
    let opens = ["fn ", "view ", "test ", "derive ", "if ", "else", "for ", "while ", "struct "].iter().any(|k| last.starts_with(k));
    if opens {
        json!({"status": "incomplete", "indent": "    "})
    } else {
        json!({"status": "invalid"})
    }
}

/// `crow kernel <connection file>`: serve Jupyter until it says stop.
pub fn serve(connection_file: &str) -> Result<(), String> {
    let text = std::fs::read_to_string(connection_file).map_err(|e| format!("kernel: cannot read '{}': {}", connection_file, e))?;
    let conn: Connection = serde_json::from_str(&text).map_err(|e| format!("kernel: '{}' is not a connection file: {}", connection_file, e))?;
    if !conn.key.is_empty() && conn.signature_scheme != "hmac-sha256" {
        return Err(format!("kernel: messages are signed with hmac-sha256, not {}", conn.signature_scheme));
    }
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().map_err(|e| format!("kernel: {}", e))?;
    rt.block_on(run(conn))
}

async fn run(conn: Connection) -> Result<(), String> {
    use zeromq::{Socket, SocketRecv, SocketSend};
    let bound = |what: &str, e: zeromq::ZmqError| format!("kernel: cannot listen for {} on {}: {}", what, conn.ip, e);
    let mut shell = zeromq::RouterSocket::new();
    shell.bind(&conn.endpoint(conn.shell_port)).await.map_err(|e| bound("shell", e))?;
    let mut control = zeromq::RouterSocket::new();
    control.bind(&conn.endpoint(conn.control_port)).await.map_err(|e| bound("control", e))?;
    let mut stdin = zeromq::RouterSocket::new();
    stdin.bind(&conn.endpoint(conn.stdin_port)).await.map_err(|e| bound("stdin", e))?;
    let mut iopub = zeromq::PubSocket::new();
    iopub.bind(&conn.endpoint(conn.iopub_port)).await.map_err(|e| bound("iopub", e))?;
    let mut hb = zeromq::RepSocket::new();
    hb.bind(&conn.endpoint(conn.hb_port)).await.map_err(|e| bound("heartbeat", e))?;
    let key = conn.key.clone();

    // every broadcast, in the order it was made
    let (io_tx, mut io_rx) = tokio::sync::mpsc::unbounded_channel::<Message>();
    let io_key = key.clone();
    tokio::spawn(async move {
        while let Some(m) = io_rx.recv().await {
            if let Some(msg) = to_zmq(m.frames(&io_key)) {
                let _ = iopub.send(msg).await;
            }
        }
    });
    // the heartbeat: each ping echoed
    tokio::spawn(async move {
        while let Ok(m) = hb.recv().await {
            if hb.send(m).await.is_err() {
                break;
            }
        }
    });
    // the cells, on the thread that holds the session
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (jobs_tx, jobs_rx) = std::sync::mpsc::channel::<Job>();
    {
        let (io, c) = (io_tx.clone(), cancel.clone());
        std::thread::spawn(move || run_cells(jobs_rx, io, c));
    }
    // control: an interrupt or a shutdown reaches the kernel while a cell runs
    {
        let (key, cancel) = (key.clone(), cancel.clone());
        tokio::spawn(async move {
            while let Ok(m) = control.recv().await {
                let Ok(req) = Message::parse(&from_zmq(m), &key) else { continue };
                let (content, stop) = match req.msg_type() {
                    "interrupt_request" => {
                        cancel.store(true, std::sync::atomic::Ordering::SeqCst);
                        (json!({"status": "ok"}), false)
                    }
                    "shutdown_request" => (json!({"status": "ok", "restart": req.content.get("restart").cloned().unwrap_or(json!(false))}), true),
                    "kernel_info_request" => (kernel_info(), false),
                    _ => continue,
                };
                let answer = reply(&req, &req.msg_type().replace("_request", "_reply"), content, req.identities.clone());
                if let Some(msg) = to_zmq(answer.frames(&key)) {
                    let _ = control.send(msg).await;
                }
                if stop {
                    std::process::exit(0);
                }
            }
        });
    }
    let _ = &mut stdin;
    // shell: each request in turn
    loop {
        let m = shell.recv().await.map_err(|e| format!("kernel: {}", e))?;
        // a message not signed with the key is not Jupyter's: dropped
        let Ok(req) = Message::parse(&from_zmq(m), &key) else { continue };
        let _ = io_tx.send(status(&req, "busy"));
        let code = req.content.get("code").and_then(Value::as_str).unwrap_or("").to_string();
        let (content, stop) = match req.msg_type() {
            "kernel_info_request" => (Some(kernel_info()), false),
            "execute_request" => {
                let silent = req.content.get("silent").and_then(Value::as_bool).unwrap_or(false);
                let (done, answer) = tokio::sync::oneshot::channel();
                if jobs_tx.send(Job { code, request: req.clone(), silent, done }).is_err() {
                    return Err("kernel: the session has stopped".to_string());
                }
                (Some(answer.await.unwrap_or_else(|_| json!({"status": "error", "ename": "CrowError", "evalue": "the session stopped", "traceback": []}))), false)
            }
            "is_complete_request" => (Some(is_complete(&code)), false),
            "complete_request" => {
                let at = req.content.get("cursor_pos").cloned().unwrap_or(json!(0));
                (Some(json!({"status": "ok", "matches": [], "cursor_start": at, "cursor_end": at, "metadata": {}})), false)
            }
            "inspect_request" => (Some(json!({"status": "ok", "found": false, "data": {}, "metadata": {}})), false),
            "history_request" => (Some(json!({"status": "ok", "history": []})), false),
            "comm_info_request" => (Some(json!({"status": "ok", "comms": {}})), false),
            "shutdown_request" => (Some(json!({"status": "ok", "restart": req.content.get("restart").cloned().unwrap_or(json!(false))})), true),
            _ => (None, false),
        };
        if let Some(content) = content {
            let answer = reply(&req, &req.msg_type().replace("_request", "_reply"), content, req.identities.clone());
            if let Some(msg) = to_zmq(answer.frames(&key)) {
                let _ = shell.send(msg).await;
            }
        }
        let _ = io_tx.send(status(&req, "idle"));
        if stop {
            // (the idle broadcast leaves before the process does)
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            std::process::exit(0);
        }
    }
}

/// `crow kernel install`: the kernel spec where Jupyter finds it. Where it
/// went.
pub fn install() -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| format!("kernel: {}", e))?;
    let dir = kernel_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("kernel: cannot write {}: {}", dir.display(), e))?;
    let spec = serde_json::to_string_pretty(&kernel_spec(&exe.display().to_string())).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("kernel.json"), spec).map_err(|e| format!("kernel: cannot write {}: {}", dir.display(), e))?;
    Ok(dir.display().to_string())
}

/// `crow kernel install`, or `crow kernel <connection file>` as Jupyter
/// starts it.
pub fn cmd_kernel(args: &[String]) {
    let outcome = match args.get(2).map(String::as_str) {
        Some("install") => install().map(|dir| println!("installed the Crow kernel in {} — Jupyter now offers Crow", dir)),
        Some(file) => serve(file),
        None => Err("usage: crow kernel install | crow kernel <connection file>".to_string()),
    };
    if let Err(e) = outcome {
        eprintln!("{}", e);
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(msg_type: &str) -> Message {
        Message {
            identities: vec![b"peer".to_vec()],
            header: json!({"msg_id": "1", "session": "s", "msg_type": msg_type, "version": "5.3"}),
            parent: json!({}),
            metadata: json!({}),
            content: json!({"code": "1 + 1"}),
        }
    }

    /// A message goes out signed with the key and comes back as it was; a
    /// message signed with another key, or not at all, is refused
    #[test]
    fn a_message_is_signed_and_checked() {
        let m = request("execute_request");
        let frames = m.frames("key");
        assert_eq!(frames[0], b"peer");
        assert_eq!(frames[1], b"<IDS|MSG>");
        let back = Message::parse(&frames, "key").unwrap_or_else(|e| panic!("{}", e));
        assert_eq!(back.msg_type(), "execute_request");
        assert_eq!(back.identities, vec![b"peer".to_vec()]);
        assert!(Message::parse(&frames, "other").is_err());
        let mut forged = frames.clone();
        forged[6] = br#"{"code": "rm['/']"}"#.to_vec();
        assert!(Message::parse(&forged, "key").is_err());
        // an empty key signs nothing, and checks nothing
        assert!(Message::parse(&m.frames(""), "").is_ok());
    }

    /// A reply is to its request: in its session, its header the parent
    #[test]
    fn a_reply_names_its_request() {
        let req = request("kernel_info_request");
        let r = reply(&req, "kernel_info_reply", kernel_info(), req.identities.clone());
        assert_eq!(r.header["msg_type"], "kernel_info_reply");
        assert_eq!(r.header["session"], "s");
        assert_eq!(r.parent["msg_id"], "1");
        assert_eq!(r.content["language_info"]["file_extension"], ".co");
    }

    #[test]
    fn a_cell_is_complete_when_it_builds() {
        assert_eq!(is_complete("x = 1\nlog[x]")["status"], "complete");
        assert_eq!(is_complete("fn f[x]")["status"], "incomplete");
        assert_eq!(is_complete("x = = 1")["status"], "invalid");
    }

    #[test]
    fn the_kernel_spec_starts_this_crow() {
        let spec = kernel_spec("/opt/crow/bin/crow");
        assert_eq!(spec["argv"], json!(["/opt/crow/bin/crow", "kernel", "{connection_file}"]));
        assert_eq!(spec["language"], "crow");
    }

    #[test]
    fn a_frame_is_a_table_its_text_escaped() {
        use crow_runtime::frame::{Column, CrowFrame};
        let f = CrowFrame::new(vec!["a<b".into()], vec![Column::strs(vec!["<x>".into(), "y".into()])], vec![vec![true, false]]);
        let html = frame_html(&f);
        assert!(html.starts_with("<table><thead><tr><th>a&lt;b</th></tr></thead><tbody><tr><td>&lt;x&gt;</td></tr>"), "{}", html);
    }

    /// What a cell writes to stdout reaches the capture as it is written,
    /// and stdout is the process's own again after
    #[cfg(unix)]
    #[test]
    fn a_cells_output_is_captured() {
        use std::io::Write;
        let got = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let kept = got.clone();
        let capture = Capture::start(std::sync::Arc::new(move |name, text| {
            if name == "stdout" {
                kept.lock().unwrap_or_else(|e| e.into_inner()).push_str(&text);
            }
        }))
        .unwrap_or_else(|e| panic!("{}", e));
        // (straight to the descriptor: the test harness keeps println!'s)
        let mut out = std::io::stdout();
        let _ = out.write_all("héllo\n".as_bytes());
        let _ = out.flush();
        capture.finish();
        // (the capture is the process's: the test harness's own lines,
        // written from its thread meanwhile, may be among what it read)
        let got = got.lock().unwrap_or_else(|e| e.into_inner()).clone();
        assert!(got.contains("héllo\n"), "{:?}", got);
    }
}
