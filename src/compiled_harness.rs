//! `crow test --compiled`: the app a test block opens is the compiled
//! program. The block runs in the interpreter, as every block does; a run
//! that opens an app with `app[view]` starts the compiled program fresh, as
//! the interpreter runs the declarations fresh for each run, and drives it
//! through `crow_web::live::test_control`. Each harness verb is one request,
//! answered once what it started has settled.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::OnceLock;

static BINARY: OnceLock<PathBuf> = OnceLock::new();

/// How many runs started the compiled program.
static STARTED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// How many runs started the compiled program so far.
pub fn started() -> usize {
    STARTED.load(std::sync::atomic::Ordering::SeqCst)
}

/// Whether any test block opens an app (`app[view]`): the only thing the
/// compiled program is started for.
pub fn opens_an_app(stmts: &[crate::ast::Stmt]) -> bool {
    use crate::ast::{Expr, Stmt};
    fn expr(e: &Expr) -> bool {
        match e {
            Expr::Call { callee, args } => matches!(callee.as_ref(), Expr::Ident(n) if n == "app") || expr(callee) || args.iter().any(expr),
            Expr::BinOp { left, right, .. } => expr(left) || expr(right),
            Expr::UnaryOp { expr: e, .. } | Expr::Named { value: e, .. } => expr(e),
            Expr::FieldAccess { object, .. } => expr(object),
            Expr::Array(items) => items.iter().any(expr),
            Expr::Map(pairs) => pairs.iter().any(|(_, v)| expr(v)),
            _ => false,
        }
    }
    fn stmts_open(stmts: &[Stmt], in_test: bool) -> bool {
        stmts.iter().any(|s| match s {
            Stmt::TestBlock { body, .. } => stmts_open(body, true),
            Stmt::Assign { value, .. } if in_test => expr(value),
            Stmt::ExprStmt(e) | Stmt::Return(Some(e)) if in_test => expr(e),
            Stmt::If { then_body, else_body, .. } if in_test => stmts_open(then_body, true) || else_body.as_ref().map_or(false, |b| stmts_open(b, true)),
            Stmt::For { body, .. } | Stmt::While { body, .. } if in_test => stmts_open(body, true),
            _ => false,
        })
    }
    stmts_open(stmts, false)
}

/// The compiled program every run that opens an app starts.
pub fn use_binary(path: PathBuf) {
    let _ = BINARY.set(path);
}

/// Whether test blocks drive the compiled program.
pub fn active() -> bool {
    BINARY.get().is_some()
}

/// A running copy of the compiled program, stopped when the run ends.
pub struct RemoteApp {
    port: u16,
    child: std::process::Child,
    /// What the program writes to stderr while it runs.
    log: PathBuf,
}

impl RemoteApp {
    /// The program started on a port of its own, once it accepts
    /// connections.
    pub fn start() -> Result<RemoteApp, String> {
        let bin = BINARY.get().ok_or("no compiled program to test")?;
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .and_then(|l| l.local_addr())
            .map(|a| a.port())
            .map_err(|e| format!("no free port for the compiled program: {}", e))?;
        let log = std::env::temp_dir().join(format!("crow_test_{}_{}.log", std::process::id(), port));
        let err = std::fs::File::create(&log).map_err(|e| format!("cannot write {}: {}", log.display(), e))?;
        let mut child = std::process::Command::new(bin)
            .env("CROW_TEST_CONTROL", "1")
            .env("CROW_PORT", port.to_string())
            .stdout(std::process::Stdio::null())
            .stderr(err)
            .spawn()
            .map_err(|e| format!("cannot start {}: {}", bin.display(), e))?;
        for _ in 0..1500 {
            if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
                STARTED.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                return Ok(RemoteApp { port, child, log });
            }
            if let Ok(Some(status)) = child.try_wait() {
                let said = std::fs::read_to_string(&log).unwrap_or_default();
                return Err(format!("the compiled program stopped before it served ({}): {}", status, said.trim()));
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let _ = child.kill();
        Err("the compiled program did not start serving within 15 seconds".into())
    }

    /// One verb: the request's answer, or the program's refusal.
    pub fn post(&self, op: &str, body: serde_json::Value) -> Result<serde_json::Value, String> {
        let mut s = std::net::TcpStream::connect(("127.0.0.1", self.port)).map_err(|e| format!("the compiled program is gone: {}", e))?;
        let text = body.to_string();
        write!(
            s,
            "POST /__crow/test/{} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            op,
            text.len(),
            text
        )
        .map_err(|e| e.to_string())?;
        let mut raw = String::new();
        s.read_to_string(&mut raw).map_err(|e| e.to_string())?;
        let answer = raw.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or("");
        let v: serde_json::Value = match serde_json::from_str(answer) {
            Ok(v) => v,
            Err(_) => {
                // what the program said as it failed, a panic's message
                // among it: the answer alone is empty when a request dies
                let said = std::fs::read_to_string(&self.log).unwrap_or_default();
                let last: Vec<&str> = said.lines().filter(|l| !l.trim().is_empty()).collect();
                let tail = last[last.len().saturating_sub(4)..].join(" | ");
                let first = raw.lines().next().unwrap_or("");
                return Err(format!(
                    "the compiled program answered {} with: {}{}",
                    op,
                    if first.is_empty() { "nothing" } else { first },
                    if tail.is_empty() { String::new() } else { format!(" — it said: {}", tail) }
                ));
            }
        };
        if let Some(e) = v.get("error").and_then(|e| e.as_str()) {
            return Err(e.to_string());
        }
        Ok(v)
    }
}

impl Drop for RemoteApp {
    fn drop(&mut self) {
        // asked to stop first, as a signal stops a served program, so it
        // ends the database work it started; then made to
        #[cfg(unix)]
        {
            let _ = std::process::Command::new("kill").arg("-TERM").arg(self.child.id().to_string()).status();
            for _ in 0..200 {
                if let Ok(Some(_)) = self.child.try_wait() {
                    let _ = std::fs::remove_file(&self.log);
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.log);
    }
}
