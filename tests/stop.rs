//! `crow stop` from the outside: a program the interpreter serves — `crow
//! run -i`, and `crow dev`'s watcher with the program it serves — is
//! stopped from the project's directory, as a compiled app is (the
//! interpreter is crow itself, so neither the pidfile nor the build's name
//! had found it, and `crow stop` had said nothing was running); another
//! project's app, and a process a stale pidfile names, are never touched.

use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// A page's markup, or nothing while no program answers.
fn get(port: u16, path: &str) -> String {
    let Ok(mut s) = std::net::TcpStream::connect(("127.0.0.1", port)) else { return String::new() };
    let _ = write!(s, "GET {} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n", path, port);
    let mut out = String::new();
    let _ = s.read_to_string(&mut out);
    out
}

fn until(what: &str, mut f: impl FnMut() -> bool) {
    let start = Instant::now();
    while !f() {
        assert!(start.elapsed() < Duration::from_secs(60), "waited a minute for {}", what);
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// A child killed when the test ends however it ends.
struct Kept(Child);
impl Drop for Kept {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn project(name: &str, port: u16) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("crow_stop_{}_{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let program = format!("use web, ui\nn = 0\nview home[]\n    return stack[p['hello `n`']]\nserve[{}]\npage['/', home]\n", port);
    std::fs::write(dir.join("app.co"), program).unwrap();
    // the directory as the process will see it (a Mac's /var is /private/var)
    dir.canonicalize().unwrap()
}

fn crow(dir: &std::path::Path, args: &[&str]) -> Kept {
    Kept(Command::new(env!("CARGO_BIN_EXE_crow")).args(args).current_dir(dir).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap())
}

fn stop(dir: &std::path::Path) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_crow")).arg("stop").current_dir(dir).output().unwrap();
    format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))
}

fn ended(c: &mut Kept) -> bool {
    matches!(c.0.try_wait(), Ok(Some(_)))
}

#[test]
fn crow_stop_stops_a_program_the_interpreter_serves_and_only_this_projects() {
    let port = 3441;
    let here = project("served", port);
    let elsewhere = project("other", 3442);
    let mut served = crow(&here, &["run", "-i", "app.co"]);
    until("the program serving", || get(port, "/").contains("hello 0"));
    // another project's directory: nothing of this one is touched
    let said = stop(&elsewhere);
    assert!(said.contains("nothing running"), "{}", said);
    assert!(get(port, "/").contains("hello 0") && !ended(&mut served), "another project's stop left it serving");
    let said = stop(&here);
    assert!(said.contains("stopped"), "{}", said);
    until("the program to end", || ended(&mut served));
    assert!(get(port, "/").is_empty(), "nothing answers once it stopped");
    let _ = std::fs::remove_dir_all(&here);
    let _ = std::fs::remove_dir_all(&elsewhere);
}

#[test]
fn crow_stop_stops_crow_dev_and_the_program_it_serves() {
    let port = 3443;
    let here = project("dev", port);
    let mut dev = crow(&here, &["dev", "app.co"]);
    until("crow dev serving", || get(port, "/").contains("hello 0"));
    let said = stop(&here);
    assert!(said.contains("stopped (2 processes)"), "the watcher and the program it serves: {}", said);
    until("crow dev to end", || ended(&mut dev));
    until("the program it served to end", || get(port, "/").is_empty());
    let _ = std::fs::remove_dir_all(&here);
}

#[test]
fn a_stale_pidfile_naming_another_process_is_not_signalled() {
    let here = project("stale", 3444);
    // a process of another directory that took the number a pidfile holds
    let mut other = Kept(Command::new("sleep").arg("30").current_dir(std::env::temp_dir()).spawn().unwrap());
    std::fs::create_dir_all(here.join(".crow")).unwrap();
    std::fs::write(here.join(".crow").join("app.pid"), other.0.id().to_string()).unwrap();
    let said = stop(&here);
    assert!(said.contains("nothing running"), "{}", said);
    std::thread::sleep(Duration::from_millis(300));
    assert!(!ended(&mut other), "the other process was not signalled");
    assert!(!here.join(".crow").join("app.pid").exists(), "the stale pidfile is let go");
    let _ = std::fs::remove_dir_all(&here);
}
