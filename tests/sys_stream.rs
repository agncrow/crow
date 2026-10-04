//! The standard library's own layer from the outside, in both modes:
//! `sys.open` reaches a host, `send` and `recv` move bytes, a server that
//! closed reads as no bytes, a closed stream refuses every call, and a
//! host nobody answers is the call's error value — the same lines from
//! `crow run -i` and from the compiled program, through the harness's
//! door (`CROW_SYS=1`), which a program has not: without it, `use sys` is
//! refused before the program runs.

use std::io::{Read, Write};
use std::process::Command;

/// A server that echoes the first bytes each connection sends, then closes it.
fn echo_server() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for mut s in listener.incoming().flatten() {
            let mut buf = [0u8; 256];
            if let Ok(n) = s.read(&mut buf) {
                let _ = s.write_all(&buf[..n]);
            }
        }
    });
    port
}

const PROGRAM: &str = "use sys

port = int[args[][0]]
s = open['127.0.0.1', port, false, 2.0]
log[s.ok]
log[type[s]]
log[send[s, bytes['ping']]]
log[text[recv[s, 64, 2.0]]]
log[len[recv[s, 64, 2.0]]]
close[s]
late = send[s, bytes['x']]
log[late.ok]
log[late.reason]
nobody = open['127.0.0.1', 1, false, 0.2]
log[nobody.ok]
log[nobody.kind]
b = bytes['héllo']
log[len[b]]
log[text[slice[b, 3, 6]]]
log[find[b, bytes['llo']]]
log[b eq bytes['héllo']]
";

fn crow() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_crow"))
}

fn run(dir: &std::path::Path, mode: &[&str], port: u16, door: bool) -> (bool, String, String) {
    let mut cmd = Command::new(crow());
    cmd.args(mode).arg(dir.join("main.co")).arg(port.to_string()).current_dir(dir);
    if door {
        cmd.env("CROW_SYS", "1");
    } else {
        cmd.env_remove("CROW_SYS");
    }
    let out = cmd.output().unwrap();
    (out.status.success(), String::from_utf8_lossy(&out.stdout).to_string(), String::from_utf8_lossy(&out.stderr).to_string())
}

#[test]
fn a_stream_works_the_same_in_both_modes_and_a_program_may_not_open_it() {
    let port = echo_server();
    let dir = std::env::temp_dir().join(format!("crow_sys_stream_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("crowfile"), "name = 'sysstream'\n").unwrap();
    std::fs::write(dir.join("main.co"), PROGRAM).unwrap();
    let (i_ok, i_out, i_err) = run(&dir, &["run", "-i"], port, true);
    assert!(i_ok, "interpreted: {}", i_err);
    let expected = "true\nstream\n4\nping\n0\nfalse\nthe stream is closed\nfalse\nsys\n6\nllo\n3\ntrue\n";
    assert_eq!(i_out, expected);
    let (c_ok, c_out, c_err) = run(&dir, &["run"], port, true);
    assert!(c_ok, "compiled: {}", c_err);
    assert_eq!(c_out, i_out, "the two modes differ");
    // the door closed: a program may not import sys
    let (p_ok, _, p_err) = run(&dir, &["run", "-i"], port, false);
    assert!(!p_ok);
    assert!(p_err.contains("`sys` is the standard library's own layer"), "{}", p_err);
    let _ = std::fs::remove_dir_all(&dir);
}
