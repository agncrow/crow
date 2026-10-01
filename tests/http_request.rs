//! A program's requests to another server, in both modes (spec, Requests):
//! `request[url, method, body]` sends a GET, a POST, a PUT, a PATCH or a
//! DELETE, with headers, a query, a form, files and a timeout when told,
//! and answers a record — its status, its body, its headers — whatever the
//! status; a JSON body goes as JSON, a form URL-encoded, files as a
//! multipart body, and a success's bytes can be saved to a file. A server
//! that does not answer (in time), a method that is none, a body on a GET
//! and a body beside a form are the call's error, of kind 'http'. The
//! readers take a URL's headers, query and timeout too.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::Command;

/// One request answered: what it was asked — the method, the path, the
/// body's type, the authorization and the body (a multipart boundary
/// written `B`, a line break `|`) — 201 for a POST, 204 (and nothing) for a
/// DELETE, 404 at /missing, the bytes 0 to 255 at /image, an answer three
/// seconds late at /slow, headers in time and the body three seconds late
/// at /stall, 200 otherwise.
fn answer(stream: std::net::TcpStream) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut first = String::new();
    let _ = reader.read_line(&mut first);
    let (mut length, mut kind, mut auth) = (0usize, String::new(), String::new());
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
            break;
        }
        let lower = line.to_ascii_lowercase();
        if let Some(v) = lower.strip_prefix("content-length:") {
            length = v.trim().parse().unwrap_or(0);
        } else if let Some(v) = line.split_once(':').filter(|_| lower.starts_with("content-type:")) {
            kind = v.1.trim().to_string();
        } else if let Some(v) = line.split_once(':').filter(|_| lower.starts_with("authorization:")) {
            auth = v.1.trim().to_string();
        }
    }
    let mut body = vec![0u8; length];
    let _ = reader.read_exact(&mut body);
    let mut body = String::from_utf8_lossy(&body).to_string();
    if let Some((_, boundary)) = kind.clone().split_once("boundary=") {
        kind = kind.replace(boundary, "B");
        body = body.replace(boundary, "B");
    }
    let body = body.replace("\r\n", "|");
    let mut words = first.split_whitespace();
    let (method, path) = (words.next().unwrap_or("").to_string(), words.next().unwrap_or("").to_string());
    if path == "/stall" {
        let mut out = stream;
        let _ = write!(out, "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 10\r\nConnection: close\r\n\r\npart");
        let _ = out.flush();
        std::thread::sleep(std::time::Duration::from_secs(3));
        let _ = out.write_all(b"ial!!!");
        return;
    }
    let said = format!("{} {} type={} auth={} body={}", method, path, kind, auth, body);
    let (status, said): (&str, Vec<u8>) = match (method.as_str(), path.as_str()) {
        (_, "/missing") => ("404 Not Found", b"no such thing".to_vec()),
        (_, "/image") => ("200 OK", (0..=255u8).collect()),
        (_, "/slow") => {
            std::thread::sleep(std::time::Duration::from_secs(3));
            ("200 OK", b"late".to_vec())
        }
        ("POST", _) => ("201 Created", said.into_bytes()),
        ("DELETE", _) => ("204 No Content", Vec::new()),
        _ => ("200 OK", said.into_bytes()),
    };
    let mut out = stream;
    let _ = write!(out, "HTTP/1.1 {}\r\nContent-Type: text/plain\r\nX-Echo: yes\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", status, said.len());
    let _ = out.write_all(&said);
}

/// A server that answers each request on a thread of its own (a slow one
/// holds up no other).
fn echo_server() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            std::thread::spawn(move || answer(stream));
        }
    });
    port
}

const PROGRAM: &str = "url = args[][0]
order = map[name = 'crow', side = 'buy']
r = request['`url`/orders', 'POST', encode[order], headers = map[authorization = 'Bearer k']]
log[r.status]
log[r.body]
log[r.headers['x-echo']]
g = request['`url`/orders/1']
log[g.status]
log[g.body]
p = request['`url`/orders/1', 'PUT', 'plain words']
log[p.body]
q = request['`url`/orders/1', 'patch', '[1, 2]']
log[q.body]
d = request['`url`/orders/1', 'DELETE']
log[d.status]
log[len[d.body]]
m = request['`url`/missing']
log[m.status]
log[m.ok]
bad = request['`url`/x', 'FETCH']
log[bad.reason]
withbody = request['`url`/x', 'GET', 'oops']
log[withbody.reason]
nobody = request['http://127.0.0.1:1/x']
log[nobody.ok]
log[nobody.kind]
s = request['`url`/search', query = map[q = 'hello world & more', page = '2']]
log[s.body]
f = request['`url`/login', 'POST', form = map[user = 'ada', pass = 'p&ss w']]
log[f.body]
u = request['`url`/upload', 'POST', form = map[title = 'notes'], files = map[doc = 'hello.txt']]
log[u.body]
pair = encode['ada:secret', 'base64']
basic = request['`url`/me', headers = map[authorization = 'Basic `pair`']]
log[basic.body]
img = request['`url`/image', save = 'got.bin']
log[img.status]
log[len[img.body]]
lost = request['`url`/missing', save = 'never.bin']
log[lost.body]
log[exists['never.bin']]
slow = request['`url`/slow', timeout = 0.5]
log[slow.reason]
both = request['`url`/x', 'POST', 'words', form = map[a = '1']]
log[both.reason]
getform = request['`url`/x', form = map[a = '1']]
log[getform.reason]
found = read['`url`/search', query = map[q = 'a b']]
log[found]
slowread = read['`url`/slow', timeout = 0.5]
log[slowread.reason]
local = read['hello.txt', timeout = 1]
log[local.reason]
stalled = read['`url`/stall', timeout = 0.5]
log[stalled.reason]
stalledreq = request['`url`/stall', timeout = 0.5]
log[stalledreq.reason]
";

#[test]
fn a_program_requests_another_server_in_both_modes() {
    let port = echo_server();
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/crow_http_request_app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.co"), PROGRAM).unwrap();
    std::fs::write(dir.join("hello.txt"), "hi there").unwrap();
    let url = format!("http://127.0.0.1:{}", port);
    let said = format!(
        "201\n\
POST /orders type=application/json auth=Bearer k body={{\"name\":\"crow\",\"side\":\"buy\"}}\n\
yes\n\
200\n\
GET /orders/1 type= auth= body=\n\
PUT /orders/1 type=text/plain; charset=utf-8 auth= body=plain words\n\
PATCH /orders/1 type=application/json auth= body=[1, 2]\n\
204\n\
0\n\
404\n\
true\n\
'FETCH' is not an HTTP method — GET, POST, PUT, PATCH, DELETE, HEAD, OPTIONS\n\
a GET request sends no body — send it with POST, PUT or PATCH\n\
false\n\
http\n\
GET /search?page=2&q=hello%20world%20%26%20more type= auth= body=\n\
POST /login type=application/x-www-form-urlencoded auth= body=pass=p%26ss%20w&user=ada\n\
POST /upload type=multipart/form-data; boundary=B auth= body=--B|Content-Disposition: form-data; name=\"title\"||notes|--B|Content-Disposition: form-data; name=\"doc\"; filename=\"hello.txt\"|Content-Type: text/plain||hi there|--B--|\n\
GET /me type= auth=Basic YWRhOnNlY3JldA== body=\n\
200\n\
0\n\
no such thing\n\
false\n\
cannot request '{url}/slow': no answer in 0.5 s\n\
a request sends a body, or a form and its files — not both\n\
a GET request sends no body — send it with POST, PUT or PATCH, or its fields as query = map[…]\n\
GET /search?q=a%20b type= auth= body=\n\
cannot read '{url}/slow': no answer in 0.5 s\n\
cannot read 'hello.txt' with headers, a query or a timeout: they go with an http:// or https:// URL\n\
cannot read '{url}/stall': no answer in 0.5 s\n\
cannot request '{url}/stall': no answer in 0.5 s\n"
    );
    for mode in [&["run", "-i"][..], &["run"][..]] {
        let _ = std::fs::remove_file(dir.join("got.bin"));
        let out = Command::new(env!("CARGO_BIN_EXE_crow")).args(mode).arg("main.co").arg(&url).current_dir(&dir).env("CROW_BUILD_PROFILE", "dev").output().unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), said, "{:?}: {}", mode, String::from_utf8_lossy(&out.stderr));
        // the answer's bytes, as they came
        assert_eq!(std::fs::read(dir.join("got.bin")).unwrap(), (0..=255u8).collect::<Vec<u8>>(), "{:?}", mode);
    }
}
