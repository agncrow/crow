//! An S3 object is read and written through the protocol written in Crow
//! (src/modules/sys.co): at an S3-compatible endpoint the environment
//! names it is reached by path and signed with the environment's
//! credentials (Signature Version 4), its bytes the request's body when
//! written; with no credentials it is read unsigned, as a public bucket
//! is; a path that is no object is the reason. Both modes, against a
//! server this test runs that answers each request in turn.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::Command;

/// A server answering each request with the next canned answer, keeping
/// every request's head (and body) as it came.
fn serve(answers: Vec<String>) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = format!("http://{}", listener.local_addr().unwrap());
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let kept = seen.clone();
    std::thread::spawn(move || {
        let mut answers = answers.into_iter();
        for stream in listener.incoming().flatten() {
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut head = String::new();
            let mut length = 0usize;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = v.trim().parse().unwrap_or(0);
                }
                head.push_str(&line);
            }
            let mut body = vec![0u8; length];
            let _ = reader.read_exact(&mut body);
            kept.lock().unwrap().push(format!("{}{}", head, String::from_utf8_lossy(&body)));
            let answer = answers.next().unwrap_or_else(|| "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string());
            let mut out = stream;
            let _ = out.write_all(answer.as_bytes());
        }
    });
    (addr, seen)
}

fn answer(status: &str, media: &str, body: &str) -> String {
    format!("HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", status, media, body.len(), body)
}

const PROGRAM: &str = "got = read['s3://bucket/2026/07/trades one.csv']
log[got]
kept = write['s3://bucket/out.parquet', 'PAR1']
log[kept.ok]
open = read['s3://open/x.csv']
log[open]
bad = read['s3://bucket-only']
log[bad.reason]
denied = read['s3://bucket/secret.csv']
log[denied.reason]
";

#[test]
fn an_s3_object_is_signed_and_reached_by_path_at_an_endpoint_in_both_modes() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/crow_s3_endpoint_app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.co"), PROGRAM).unwrap();
    for mode in [&["run", "-i"][..], &["run"][..]] {
        // one server per mode: each answers the program's five requests in turn
        // (the public read goes unsigned: the program's environment loses its credentials between runs)
        let (addr, seen) = serve(vec![
            answer("200 OK", "text/csv", "a,b\n1,2\n"),
            answer("200 OK", "application/xml", ""),
            answer("200 OK", "text/csv", "x\n"),
            answer("403 Forbidden", "application/xml", "<Error><Code>AccessDenied</Code><Message>Access Denied</Message></Error>"),
        ]);
        let out = Command::new(env!("CARGO_BIN_EXE_crow"))
            .args(mode)
            .arg("main.co")
            .current_dir(&dir)
            .env("CROW_BUILD_PROFILE", "dev")
            .env("AWS_ENDPOINT_URL", &addr)
            .env("AWS_ACCESS_KEY_ID", "AKIDEXAMPLE")
            .env("AWS_SECRET_ACCESS_KEY", "secret")
            .env("AWS_REGION", "eu-west-2")
            .output()
            .unwrap();
        let said = "a,b\n1,2\n\ntrue\nx\n\ncannot read 's3://bucket-only': an S3 path is s3://bucket/key\ncannot read 's3://bucket/secret.csv': the server answered 403 Forbidden — Access Denied\n";
        assert_eq!(String::from_utf8_lossy(&out.stdout), said, "{:?}: {}", mode, String::from_utf8_lossy(&out.stderr));
        let seen = seen.lock().unwrap();
        assert!(seen[0].starts_with("GET /bucket/2026/07/trades%20one.csv HTTP/1.1"), "{}", seen[0]);
        let lower = seen[0].to_ascii_lowercase();
        assert!(lower.contains("authorization: aws4-hmac-sha256 credential=akidexample/"), "{}", seen[0]);
        assert!(lower.contains("/eu-west-2/s3/aws4_request, signedheaders=host;x-amz-content-sha256;x-amz-date, signature="), "{}", seen[0]);
        assert!(lower.contains("x-amz-content-sha256: e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"), "{}", seen[0]);
        assert!(seen[1].starts_with("PUT /bucket/out.parquet HTTP/1.1") && seen[1].ends_with("PAR1"), "{}", seen[1]);
        assert!(seen[2].starts_with("GET /open/x.csv HTTP/1.1") && seen[2].to_ascii_lowercase().contains("authorization: aws4-hmac-sha256"), "{}", seen[2]);
    }
}
