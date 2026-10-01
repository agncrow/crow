//! A server's options from the outside, in both modes: `serve[…, cors =
//! […], files = 'public']` and `route[…, limit = n]`, a form's fields and
//! its files in `req.form` and `req.files`, a query decoded — the same
//! answers from `crow run -i` and from the compiled program.

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// What the server answered to a request written whole: its status line,
/// headers and body.
fn ask(port: u16, request: &str) -> String {
    let Ok(mut s) = std::net::TcpStream::connect(("127.0.0.1", port)) else { return String::new() };
    let _ = s.set_read_timeout(Some(Duration::from_secs(10)));
    let _ = s.write_all(request.as_bytes());
    let mut out = String::new();
    let _ = s.read_to_string(&mut out);
    out
}

fn get(port: u16, path: &str, extra: &str) -> String {
    ask(port, &format!("GET {} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\n{}Connection: close\r\n\r\n", path, port, extra))
}

struct Served(std::process::Child);

impl Drop for Served {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

const PROGRAM: &str = "use web

fn hello[req]
    return text['hello `req.query.name`']

fn up[req]
    names = for f in req.files
        f.name
    shown = join[names, ',']
    title = req.form.title
    return text['`title`: `len[req.files]` file `shown` of `req.files[0].size` bytes']

fn api[req]
    return text['ok']

route['/hello', hello]
route['/up', 'POST', up]
route['/api', api, limit = 2]
serve[3000, cors = ['https://app.example'], files = 'public']
";

/// The program served in one mode, asked the same questions.
fn check(mode: &[&str], port: u16, dir: &std::path::Path) {
    let child = Command::new(env!("CARGO_BIN_EXE_crow"))
        .args(mode)
        .arg("main.co")
        .current_dir(dir)
        .env("CROW_PORT", port.to_string())
        // (a target directory the environment names elsewhere is not where
        // the build goes)
        .env("CARGO_TARGET_DIR", std::env::temp_dir().join("crow_somewhere_else"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let _served = Served(child);
    let by = Instant::now() + Duration::from_secs(600);
    let mut hello = String::new();
    while Instant::now() < by {
        hello = get(port, "/hello?name=a%20b", "");
        if hello.contains("hello") {
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    assert!(hello.ends_with("hello a b"), "{:?}: a decoded query: {}", mode, hello);
    // a file the directory holds, where no route answers
    let logo = get(port, "/logo.txt", "");
    assert!(logo.starts_with("HTTP/1.1 200") && logo.ends_with("a logo"), "{:?}: {}", mode, logo);
    assert!(get(port, "/nope.txt", "").starts_with("HTTP/1.1 404"), "{:?}", mode);
    // CORS: the site named, and no other
    let pre = ask(port, &format!("OPTIONS /api HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nOrigin: https://app.example\r\nAccess-Control-Request-Method: GET\r\nConnection: close\r\n\r\n", port)).to_ascii_lowercase();
    assert!(pre.contains("access-control-allow-origin: https://app.example"), "{:?}: {}", mode, pre);
    let other = get(port, "/hello?name=x", "Origin: https://evil.example\r\n").to_ascii_lowercase();
    assert!(!other.contains("access-control-allow-origin"), "{:?}: {}", mode, other);
    // a form and its file
    let body = "--XYZ\r\nContent-Disposition: form-data; name=\"title\"\r\n\r\nQ3\r\n--XYZ\r\nContent-Disposition: form-data; name=\"sheet\"; filename=\"t.csv\"\r\nContent-Type: text/csv\r\n\r\na,b\n1,2\n\r\n--XYZ--\r\n";
    let up = ask(port, &format!("POST /up HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Type: multipart/form-data; boundary=XYZ\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", port, body.len(), body));
    assert!(up.ends_with("Q3: 1 file t.csv of 8 bytes"), "{:?}: {}", mode, up);
    // the limit: two a minute, the third refused with when to come back
    assert!(get(port, "/api", "").ends_with("ok"), "{:?}", mode);
    assert!(get(port, "/api", "").ends_with("ok"), "{:?}", mode);
    let third = get(port, "/api", "").to_ascii_lowercase();
    assert!(third.starts_with("http/1.1 429") && third.contains("retry-after:"), "{:?}: {}", mode, third);
}

#[test]
fn a_servers_options_are_both_modes() {
    // (kept under target/, so the compiled program's build is reused)
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/crow_web_options_app");
    let _ = std::fs::remove_dir_all(dir.join("public"));
    std::fs::create_dir_all(dir.join("public")).unwrap();
    std::fs::write(dir.join("main.co"), PROGRAM).unwrap();
    std::fs::write(dir.join("public/logo.txt"), "a logo").unwrap();
    check(&["run", "-i"], 3471, &dir);
    check(&["run"], 3472, &dir);
}
