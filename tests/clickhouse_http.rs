//! The ClickHouse driver written in Crow (docs/db-in-crow-design.md), in
//! both modes, against a server of the test's own that speaks the HTTP
//! interface: a query's rows typed as the server names them, a NULL a
//! missing cell, the authorization a URL's user and password become, the
//! server's refusal in its own words, an insert sent as RowBinary encoded
//! by the table's types, `describe` with the parameters as NULL, and
//! `exec`. The connection is kept between statements.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::Command;
use std::sync::{Arc, Mutex};

/// RowBinaryWithNamesAndTypes of two columns `n UInt64` and `x Nullable(Float64)`.
fn rows_n_x() -> Vec<u8> {
    let mut b = vec![2u8, 1, b'n', 1, b'x', 6];
    b.extend_from_slice(b"UInt64");
    b.push(17);
    b.extend_from_slice(b"Nullable(Float64)");
    for (n, x) in [(1u64, Some(2.5f64)), (2, None), (3, Some(0.5))] {
        b.extend_from_slice(&n.to_le_bytes());
        match x {
            Some(x) => {
                b.push(0);
                b.extend_from_slice(&x.to_le_bytes());
            }
            None => b.push(1),
        }
    }
    b
}

/// RowBinaryWithNamesAndTypes of one column `a String` holding `text`.
fn one_text(text: &str) -> Vec<u8> {
    let mut b = vec![1u8, 1, b'a', 6];
    b.extend_from_slice(b"String");
    b.push(text.len() as u8);
    b.extend_from_slice(text.as_bytes());
    b
}

const REFUSED: &str = "Code: 60. DB::Exception: Unknown table expression identifier 'nowhere' in scope SELECT nope FROM nowhere. (UNKNOWN_TABLE) (version 25.7.2.54 (official build))\n\n0. Poco::Exception::Exception(String const&, int) @ 0x000000011\n1. DB::Exception::Exception(DB::Exception::MessageMasked&&, int, bool) @ 0x00000000f\n";

/// One connection served until the client closes it: each request's
/// statement answered as ClickHouse would.
fn serve(stream: std::net::TcpStream, inserted: Arc<Mutex<Vec<u8>>>, requests: Arc<Mutex<usize>>) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut out = stream;
    loop {
        let mut first = String::new();
        if reader.read_line(&mut first).unwrap_or(0) == 0 {
            return;
        }
        let (mut length, mut auth) = (0usize, String::new());
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                break;
            }
            let lower = line.to_ascii_lowercase();
            if let Some(v) = lower.strip_prefix("content-length:") {
                length = v.trim().parse().unwrap_or(0);
            } else if lower.starts_with("authorization:") {
                auth = line.split_once(':').map(|(_, v)| v.trim().to_string()).unwrap_or_default();
            }
        }
        let mut body = vec![0u8; length];
        let _ = reader.read_exact(&mut body);
        *requests.lock().unwrap() += 1;
        let path = first.split_whitespace().nth(1).unwrap_or("").to_string();
        let sql_end = body.iter().position(|&c| c == b'\n').unwrap_or(body.len());
        let sql = String::from_utf8_lossy(&body[..sql_end]).to_string();
        let describe = |inner: &str| -> (&'static str, Vec<u8>) {
            if inner.contains("nope") {
                ("404 Not Found", REFUSED.as_bytes().to_vec())
            } else if inner.contains("'auth'") {
                ("200 OK", b"a\tString\t\t\t\t\t\n".to_vec())
            } else {
                ("200 OK", b"n\tUInt64\t\t\t\t\t\nx\tNullable(Float64)\t\t\t\t\t\n".to_vec())
            }
        };
        let (status, answer): (&str, Vec<u8>) = if sql.starts_with("SELECT 1") || sql.starts_with("KILL QUERY") || sql.starts_with("SELECT DISTINCT query_id") {
            ("200 OK", if sql.starts_with("SELECT 1") { b"1\n".to_vec() } else { Vec::new() })
        } else if sql.starts_with("DESCRIBE TABLE t") {
            ("200 OK", b"n\tUInt64\t\t\t\t\t\nname\tNullable(String)\t\t\t\t\t\n".to_vec())
        } else if let Some(inner) = sql.strip_prefix("DESCRIBE TABLE (") {
            describe(inner)
        } else if sql.starts_with("INSERT INTO t (n, name) FORMAT RowBinary") {
            *inserted.lock().unwrap() = body[sql_end + 1..].to_vec();
            ("200 OK", Vec::new())
        } else if sql.starts_with("SELECT number AS n") && path.contains("default_format=RowBinaryWithNamesAndTypes") && path.contains("database=stuff") && path.contains("query_id=crow-") {
            ("200 OK", rows_n_x())
        } else if sql.starts_with("SELECT 'auth'") {
            ("200 OK", one_text(&auth))
        } else if sql.contains("nope") {
            ("404 Not Found", REFUSED.as_bytes().to_vec())
        } else {
            ("500 Internal Server Error", format!("Code: 62. DB::Exception: the test server does not know `{}`", sql).into_bytes())
        };
        let _ = write!(out, "HTTP/1.1 {}\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: Keep-Alive\r\nX-ClickHouse-Query-Id: test\r\n\r\n", status, answer.len());
        let _ = out.write_all(&answer);
        let _ = out.flush();
    }
}

const PROGRAM: &str = "use db, frame

port = args[][0]
ch = connect['clickhouse://alice:secret@127.0.0.1:`port`/stuff']
rows = query[ch, 'SELECT number AS n, number/7 AS x FROM numbers(3)']
log[rows]
log[int[rows.n[2]] * 2]
log[rows.x[1].ok]
log[rows.x[0].ok]
auth = query[ch, 'SELECT \\'auth\\'']
log[auth.a[0]]
bad = query[ch, 'SELECT nope FROM nowhere']
log[bad.ok]
log[bad.reason]
f = frame.new[]
f = with[f, 'n', [1, 2]]
f = with[f, 'name', ['a', 'b']]
log[insert[ch, 't', f]]
log[describe[ch, 'SELECT n, x FROM t WHERE n = ?']]
log[exec[ch, 'KILL QUERY WHERE query_id = \\'x\\'']]
log[ch]
";

#[test]
fn the_clickhouse_driver_written_in_crow_speaks_to_a_server_in_both_modes() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let inserted: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let requests: Arc<Mutex<usize>> = Arc::new(Mutex::new(0));
    {
        let (inserted, requests) = (inserted.clone(), requests.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let (i, r) = (inserted.clone(), requests.clone());
                std::thread::spawn(move || serve(stream, i, r));
            }
        });
    }
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/crow_clickhouse_http_app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.co"), PROGRAM).unwrap();
    let expected_insert = {
        use crow_runtime::frame::{Column, CrowFrame};
        let f = CrowFrame::new(vec!["n".into(), "name".into()], vec![Column::Float(vec![1.0, 2.0]), Column::strs(vec!["a".into(), "b".into()])], vec![vec![true, true], vec![true, true]]);
        crow_runtime::rowbinary::rowbinary_of_frame(&f, &["UInt64".to_string(), "Nullable(String)".to_string()]).unwrap()
    };
    let mut said: Vec<String> = Vec::new();
    for mode in [&["run", "-i"][..], &["run"][..]] {
        let out = Command::new(env!("CARGO_BIN_EXE_crow")).args(mode).arg("main.co").arg(port.to_string()).current_dir(&dir).env("CROW_BUILD_PROFILE", "dev").output().unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        assert!(out.status.success(), "{:?} failed: {}\n{}", mode, stdout, stderr);
        assert!(stdout.contains("[3 rows x 2 cols]"), "{:?}: {}", mode, stdout);
        assert!(stdout.contains("\n6\nfalse\ntrue\nBasic YWxpY2U6c2VjcmV0\nfalse\nclickhouse query error: Code: 60. DB::Exception: Unknown table expression identifier 'nowhere' in scope SELECT nope FROM nowhere. (UNKNOWN_TABLE) (version 25.7.2.54 (official build))\n2\n"), "{:?}: {}", mode, stdout);
        assert!(stdout.contains("   n     int\n   x  float?"), "describe's frame\n{:?}: {}", mode, stdout);
        assert!(stdout.ends_with("0\n<db:clickhouse>\n"), "{:?}: {}", mode, stdout);
        assert_eq!(*inserted.lock().unwrap(), expected_insert, "{:?}: the insert's RowBinary", mode);
        said.push(stdout);
    }
    assert_eq!(said[0], said[1], "the modes agree");
    // every statement of a run went down one kept connection: the check,
    // the sweep, six statements, the insert's describe
    assert!(*requests.lock().unwrap() >= 18, "{}", requests.lock().unwrap());
}
