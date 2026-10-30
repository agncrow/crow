//! The RESP door in both modes (packages/magpie, `use magpie.redis`): a
//! Crow program serving `redis.serve[root, port]` is driven by a client of this test's own
//! that speaks RESP2 as redis-cli does — SET, GET, INCR, MGET, DEL, a
//! stream added to and read — interpreted and compiled.
use std::io::{Read, Write};
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

mod common;

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn wait_for(port: u16, secs: u64) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    while std::time::Instant::now() < deadline {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    false
}

/// A request as RESP writes it, and the reply as bytes, read until a
/// whole reply is there (a simple string, an int, an error, a bulk
/// string or an array of them).
fn ask(s: &mut std::net::TcpStream, words: &[&str]) -> String {
    let mut req = format!("*{}\r\n", words.len());
    for w in words {
        req.push_str(&format!("${}\r\n{}\r\n", w.len(), w));
    }
    s.write_all(req.as_bytes()).unwrap();
    let mut have: Vec<u8> = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = s.read(&mut buf).expect("the door answers");
        if n == 0 {
            break;
        }
        have.extend_from_slice(&buf[..n]);
        if whole(&have).is_some() {
            break;
        }
    }
    String::from_utf8_lossy(&have).to_string()
}

/// How many bytes one whole reply takes, none where it is not whole yet.
fn whole(b: &[u8]) -> Option<usize> {
    let line_end = b.windows(2).position(|w| w == b"\r\n")?;
    match b[0] {
        b'+' | b'-' | b':' => Some(line_end + 2),
        b'$' => {
            let n: i64 = std::str::from_utf8(&b[1..line_end]).ok()?.parse().ok()?;
            if n < 0 {
                return Some(line_end + 2);
            }
            let total = line_end + 2 + n as usize + 2;
            (b.len() >= total).then_some(total)
        }
        b'*' => {
            let n: i64 = std::str::from_utf8(&b[1..line_end]).ok()?.parse().ok()?;
            let mut at = line_end + 2;
            for _ in 0..n.max(0) {
                at += whole(&b[at..])?;
            }
            Some(at)
        }
        _ => None,
    }
}

fn drive(interpreted: bool) {
    let dir = std::env::temp_dir().join(format!("crow-redis-{}-{}", if interpreted { "i" } else { "c" }, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    common::link_magpie(&dir);
    let port = free_port();
    std::fs::write(dir.join("door.co"), format!("use magpie.redis\n\nserved = redis.serve['store', {port}]\nlog[served]\n")).unwrap();
    let crow = env!("CARGO_BIN_EXE_crow");
    let mut cmd = Command::new(crow);
    cmd.arg("run");
    if interpreted {
        cmd.arg("-i");
    }
    let mut door = cmd.arg("door.co").current_dir(&dir).process_group(0).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().expect("the door starts");
    assert!(wait_for(port, if interpreted { 60 } else { 600 }), "the door did not listen on {port}");
    let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(std::time::Duration::from_secs(30))).unwrap();
    assert_eq!(ask(&mut s, &["PING"]), "+PONG\r\n");
    assert_eq!(ask(&mut s, &["SET", "greeting", "hello world"]), "+OK\r\n");
    assert_eq!(ask(&mut s, &["GET", "greeting"]), "$11\r\nhello world\r\n");
    assert_eq!(ask(&mut s, &["GET", "nothing"]), "$-1\r\n");
    assert_eq!(ask(&mut s, &["INCR", "hits"]), ":1\r\n");
    assert_eq!(ask(&mut s, &["INCRBY", "hits", "41"]), ":42\r\n");
    assert_eq!(ask(&mut s, &["MSET", "a", "1", "b", "2"]), "+OK\r\n");
    assert_eq!(ask(&mut s, &["MGET", "a", "b", "c"]), "*3\r\n$1\r\n1\r\n$1\r\n2\r\n$-1\r\n");
    assert_eq!(ask(&mut s, &["DEL", "a"]), ":1\r\n");
    assert_eq!(ask(&mut s, &["EXISTS", "a", "b"]), ":1\r\n");
    assert_eq!(ask(&mut s, &["XADD", "events", "*", "kind", "click", "page", "/"]), "$3\r\n0-0\r\n");
    assert_eq!(ask(&mut s, &["XADD", "events", "*", "kind", "view"]), "$3\r\n1-0\r\n");
    assert_eq!(ask(&mut s, &["XLEN", "events"]), ":2\r\n");
    assert_eq!(ask(&mut s, &["XREAD", "STREAMS", "events", "0"]), "*1\r\n*2\r\n$6\r\nevents\r\n*1\r\n*2\r\n$3\r\n1-0\r\n*2\r\n$4\r\nkind\r\n$4\r\nview\r\n");
    // a second connection sees the same store
    let mut t = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    t.set_read_timeout(Some(std::time::Duration::from_secs(30))).unwrap();
    assert_eq!(ask(&mut t, &["GET", "hits"]), "$2\r\n42\r\n");
    assert_eq!(ask(&mut t, &["DBSIZE"]), ":3\r\n");
    let _ = Command::new("kill").args(["-KILL", "--", &format!("-{}", door.id())]).status();
    let _ = door.kill();
    let _ = door.wait();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_redis_client_speaks_to_the_interpreted_door() {
    drive(true);
}

#[test]
fn a_redis_client_speaks_to_the_compiled_door() {
    drive(false);
}
