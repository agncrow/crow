//! What the integration tests share: a Postgres cluster of the test's own.

#![allow(dead_code)]

use std::process::Command;

/// A Postgres cluster of the test's own, on its own port, stopped when the
/// test ends.
pub struct Scratch {
    pub dir: std::path::PathBuf,
    pub port: u16,
}

impl Scratch {
    pub fn new(dir: std::path::PathBuf) -> Option<Scratch> {
        let found = Command::new("initdb").arg("--version").output().is_ok_and(|o| o.status.success());
        if !found {
            eprintln!("no initdb on the PATH — a test that needs Postgres is skipped");
            return None;
        }
        let _ = std::fs::remove_dir_all(&dir);
        let init = Command::new("initdb").arg("-D").arg(&dir).args(["-U", "crow", "--auth=trust"]).output().ok()?;
        assert!(init.status.success(), "initdb: {}", String::from_utf8_lossy(&init.stderr));
        let port = std::net::TcpListener::bind("127.0.0.1:0").ok()?.local_addr().ok()?.port();
        let s = Scratch { dir, port };
        s.start();
        Some(s)
    }

    pub fn start(&self) {
        let opts = format!("-p {} -c unix_socket_directories='' -c listen_addresses=127.0.0.1", self.port);
        let ok = Command::new("pg_ctl").arg("-D").arg(&self.dir).args(["-o", &opts, "-w", "-l"]).arg(self.dir.join("log")).arg("start").output().map(|o| o.status.success()).unwrap_or(false);
        assert!(ok, "the scratch cluster starts");
    }

    pub fn stop(&self) {
        let _ = Command::new("pg_ctl").arg("-D").arg(&self.dir).args(["-m", "fast", "-w", "stop"]).output();
    }

    /// Run SQL on it, as its owner (seeding a table): whether it ran.
    pub fn psql(&self, sql: &str) -> bool {
        Command::new("psql").args(["-h", "127.0.0.1", "-p", &self.port.to_string(), "-U", "crow", "-d", "postgres", "-q", "-v", "ON_ERROR_STOP=1", "-c", sql]).output().is_ok_and(|o| o.status.success())
    }

    pub fn running(&self) -> bool {
        Command::new("pg_ctl").arg("-D").arg(&self.dir).arg("status").output().is_ok_and(|o| o.status.success())
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        self.stop();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

