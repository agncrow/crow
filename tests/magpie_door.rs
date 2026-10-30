//! The HTTP door in both modes (packages/magpie, `door`): a Crow program
//! serving `magpie.door[dir, port]` is reached by a second program
//! through the same verbs given the door's URL — interpreted and
//! compiled — and by a plain HTTP client of this test's own.
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

fn http(port: u16, method: &str, target: &str, body: &str) -> (u16, String) {
    let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(std::time::Duration::from_secs(60))).unwrap();
    let req = format!("{method} {target} HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\n\r\n{body}", body.len());
    s.write_all(req.as_bytes()).unwrap();
    let mut have = Vec::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = s.read(&mut buf).expect("the door answers");
        if n == 0 {
            break;
        }
        have.extend_from_slice(&buf[..n]);
        let text = String::from_utf8_lossy(&have);
        if let Some(gap) = text.find("\r\n\r\n") {
            let len: usize = text[..gap].lines().find_map(|l| l.to_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse().unwrap_or(0))).unwrap_or(0);
            if have.len() >= gap + 4 + len {
                break;
            }
        }
    }
    let text = String::from_utf8_lossy(&have).to_string();
    let status: u16 = text.split(' ').nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    let body = text.find("\r\n\r\n").map(|g| text[g + 4..].to_string()).unwrap_or_default();
    (status, body)
}

fn drive(interpreted: bool) {
    let dir = std::env::temp_dir().join(format!("crow-magpie-door-{}-{}", if interpreted { "i" } else { "c" }, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    common::link_magpie(&dir);
    let port = free_port();
    std::fs::write(dir.join("door.co"), format!("use magpie\n\nserved = magpie.door['store', {port}]\nlog[served]\n")).unwrap();
    let crow = env!("CARGO_BIN_EXE_crow");
    let mut cmd = Command::new(crow);
    cmd.arg("run");
    if interpreted {
        cmd.arg("-i");
    }
    let mut door = cmd.arg("door.co").current_dir(&dir).process_group(0).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().expect("the door starts");
    assert!(wait_for(port, if interpreted { 60 } else { 600 }), "the door did not listen on {port}");
    // a plain client: the path names the log under the root and the verb
    let (status, body) = http(port, "POST", "/orders/batch?by=id", "[{\"id\": 1, \"qty\": 10}, {\"id\": 2, \"qty\": 20}]");
    assert_eq!((status, body.as_str()), (200, "0"));
    assert_eq!(http(port, "GET", "/orders/head", ""), (200, "1".to_string()));
    let (status, body) = http(port, "GET", "/orders/lookup?on=id&equal=2&columns=qty", "");
    assert_eq!(status, 200);
    assert!(body.contains("20"), "{body}");
    assert_eq!(http(port, "GET", "/orders/nosuch", "").0, 404);
    assert_eq!(http(port, "GET", "/head", "").0, 404);
    assert_eq!(http(port, "GET", "/../orders/head", "").0, 404);
    // a second program, through the verbs given the URL, in the other mode
    let client = dir.join("client.co");
    std::fs::write(&client, format!("use magpie\nuse frame\n\nd = 'http://127.0.0.1:{port}/orders'\nlog[magpie.head[d]]\nlog[magpie.batch[d, frame.new[id = [3], qty = [30]]]]\nlog[magpie.latest[d].qty]\nlog[magpie.lookup[d, ['id'], ['2'], ['qty']].qty]\nlog[magpie.latest[d, equal = ['3']].qty]\nlog[magpie.summary[d, [], ['qty'], ['sum']].qty_sum]\nlog[magpie.since[d, 1, 2].qty]\nlog[magpie.put[d, frame.new[id = [4], qty = [40]]]]\nlog[magpie.sync[d] gte 1]\nlog[len[magpie.merge[d]] gt 0]\nlog[magpie.batches[d].form]\nlog[magpie.lookup[d, ['nosuch'], ['1']].kind]\n")).unwrap();
    let mut client_cmd = Command::new(crow);
    client_cmd.arg("run");
    if !interpreted {
        client_cmd.arg("-i");
    }
    let out = client_cmd.arg("client.co").current_dir(&dir).output().expect("the client runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(stdout, "1\n2\n[10, 20, 30]\n[20]\n[30]\n[60]\n[20, 30]\n3\ntrue\ntrue\n['segment']\nmagpie\n", "{stderr}");
    let _ = Command::new("kill").args(["-KILL", "--", &format!("-{}", door.id())]).status();
    let _ = door.kill();
    let _ = door.wait();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_verbs_reach_the_interpreted_door_over_http() {
    drive(true);
}

#[test]
fn the_verbs_reach_the_compiled_door_over_http() {
    drive(false);
}
