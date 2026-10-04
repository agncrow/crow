//! A message is sent through SMTP by the mailer written in Crow
//! (src/modules/mail.co): the greeting read, EHLO, AUTH PLAIN with the
//! mailer's user and password, the envelope, the message with its dots
//! stuffed, QUIT — against a server this test runs, in both modes; a
//! recipient the server refuses is the reason, so is an address that is
//! none; a server that offers STARTTLS is asked to start it.

use std::io::{BufRead, BufReader, Write};
use std::process::Command;

type Seen = std::sync::Arc<std::sync::Mutex<Vec<Vec<String>>>>;

/// An SMTP server that takes every message for anyone but nobody, keeping
/// each connection's commands and the message's text; offering STARTTLS
/// when told, and then failing to start it (it has no certificate).
fn serve(starttls: bool) -> (u16, Seen) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen: Seen = Default::default();
    let kept = seen.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut out = stream.try_clone().unwrap();
            let mut reader = BufReader::new(stream);
            let mut lines: Vec<String> = Vec::new();
            let _ = out.write_all(b"220 test ESMTP\r\n");
            let mut line = String::new();
            while reader.read_line(&mut line).unwrap_or(0) > 0 {
                let command = line.trim_end_matches("\r\n").to_string();
                line.clear();
                lines.push(command.clone());
                let upper = command.to_ascii_uppercase();
                let reply: &[u8] = if upper.starts_with("EHLO") {
                    if starttls { b"250-test\r\n250-STARTTLS\r\n250 AUTH PLAIN\r\n" } else { b"250-test\r\n250 AUTH PLAIN\r\n" }
                } else if upper.starts_with("STARTTLS") {
                    let _ = out.write_all(b"220 go ahead\r\n");
                    break;
                } else if upper.starts_with("AUTH PLAIN") {
                    b"235 ok\r\n"
                } else if upper.starts_with("MAIL FROM") {
                    b"250 ok\r\n"
                } else if upper.starts_with("RCPT TO:<NOBODY@") {
                    b"550 no such user\r\n"
                } else if upper.starts_with("RCPT TO") {
                    b"250 ok\r\n"
                } else if upper == "DATA" {
                    let _ = out.write_all(b"354 go\r\n");
                    let mut text = String::new();
                    let mut l = String::new();
                    while reader.read_line(&mut l).unwrap_or(0) > 0 {
                        text.push_str(&l);
                        if text.ends_with("\r\n.\r\n") {
                            break;
                        }
                        l.clear();
                    }
                    lines.push(format!("<message>{}", text));
                    b"250 queued\r\n"
                } else if upper == "QUIT" {
                    let _ = out.write_all(b"221 bye\r\n");
                    break;
                } else {
                    b"500 what\r\n"
                };
                let _ = out.write_all(reply);
            }
            kept.lock().unwrap().push(lines);
        }
    });
    (port, seen)
}

const PROGRAM: &str = "use web, web.mail
port = int[args[][0]]
m = smtp['127.0.0.1', port, 'ada@example.com', 'pw']
log[m]
r = mail[m, 'bob@example.com', 'Hi there', 'line one\n.leading dot\nlast']
log[r.ok]
bad = mail[m, 'nobody@example.com', 'Hi', 'x']
log[bad.ok]
log[bad.reason]
odd = mail[m, 'nobody', 'Hi', 'x']
log[odd.reason]
";

const SAID: &str = "<mailer:127.0.0.1>\ntrue\nfalse\nmail to nobody@example.com failed: the server answered 550 no such user at RCPT TO\nmail to nobody failed: 'nobody' is no address (name@host)\n";

#[test]
fn a_message_is_sent_through_smtp_in_both_modes() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/crow_smtp_app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.co"), PROGRAM).unwrap();
    for mode in [&["run", "-i"][..], &["run"][..]] {
        let (port, seen) = serve(false);
        let out = Command::new(env!("CARGO_BIN_EXE_crow")).args(mode).arg("main.co").arg(port.to_string()).current_dir(&dir).env("CROW_BUILD_PROFILE", "dev").output().unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), SAID, "{:?}: {}", mode, String::from_utf8_lossy(&out.stderr));
        // the server was not asked more than the two messages the program sent to it
        std::thread::sleep(std::time::Duration::from_millis(200));
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 2, "{:?}", seen);
        let first = &seen[0];
        assert_eq!(first[0], "EHLO localhost");
        assert_eq!(first[1], "AUTH PLAIN AGFkYUBleGFtcGxlLmNvbQBwdw==");
        assert_eq!(first[2], "MAIL FROM:<ada@example.com>");
        assert_eq!(first[3], "RCPT TO:<bob@example.com>");
        assert_eq!(first[4], "DATA");
        let message = &first[5];
        assert!(message.starts_with("<message>From: <ada@example.com>\r\nTo: <bob@example.com>\r\nSubject: Hi there\r\nDate: "), "{}", message);
        assert!(message.contains("\r\nMIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: 7bit\r\n\r\nline one\r\n..leading dot\r\nlast\r\n.\r\n"), "{}", message);
        assert_eq!(first[6], "QUIT");
        assert_eq!(seen[1][3], "RCPT TO:<nobody@example.com>");
        assert!(!seen[1].iter().any(|l| l == "DATA"), "{:?}", seen[1]);
    }
}

const STARTTLS_PROGRAM: &str = "use web, web.mail
port = int[args[][0]]
m = smtp['127.0.0.1', port, 'ada@example.com', 'pw']
r = mail[m, 'bob@example.com', 'Hi', 'x']
log[r.ok]
log[starts[r.reason, 'mail to bob@example.com failed: ']]
";

#[test]
fn a_server_that_offers_starttls_is_asked_to_start_it() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/crow_smtp_starttls_app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.co"), STARTTLS_PROGRAM).unwrap();
    for mode in [&["run", "-i"][..], &["run"][..]] {
        let (port, seen) = serve(true);
        let out = Command::new(env!("CARGO_BIN_EXE_crow")).args(mode).arg("main.co").arg(port.to_string()).current_dir(&dir).env("CROW_BUILD_PROFILE", "dev").output().unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), "false\ntrue\n", "{:?}: {}", mode, String::from_utf8_lossy(&out.stderr));
        std::thread::sleep(std::time::Duration::from_millis(200));
        let seen = seen.lock().unwrap();
        assert_eq!(seen[0], vec!["EHLO localhost".to_string(), "STARTTLS".to_string()], "{:?}", seen);
    }
}
