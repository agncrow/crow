//! A WebSocket client from the outside, in both modes: `socket[url]` opens
//! a connection, `send` and `recv` talk over it, a wait that passes its
//! seconds and a server that closed are the calls' error values, a wait
//! that ends inside a message leaves the message whole for the next read,
//! and a server that does not answer is `socket`'s — the same lines from
//! `crow run -i` and from the compiled program.

use std::process::Command;

/// A server that echoes each text it is sent, sends "slow" back as a frame
/// in two writes 600 ms apart, and closes a connection that says "bye"; it
/// serves connections one after another.
fn echo_server() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let Ok(mut ws) = tungstenite::accept(stream) else { continue };
            while let Ok(msg) = ws.read() {
                if let tungstenite::Message::Text(t) = msg {
                    if t.as_str() == "slow" {
                        // (a server's frame: unmasked, its length in one byte)
                        use std::io::Write;
                        let said = b"slow and steady";
                        let mut frame = vec![0x81, said.len() as u8];
                        frame.extend_from_slice(said);
                        let raw = ws.get_mut();
                        let _ = raw.write_all(&frame[..3]);
                        let _ = raw.flush();
                        std::thread::sleep(std::time::Duration::from_millis(600));
                        let _ = raw.write_all(&frame[3..]);
                        let _ = raw.flush();
                        continue;
                    }
                    if t.as_str() == "bye" {
                        let _ = ws.close(None);
                        while ws.read().is_ok() {}
                        break;
                    }
                    let _ = ws.send(tungstenite::Message::Text(format!("echo: {}", t).into()));
                }
            }
        }
    });
    port
}

const PROGRAM: &str = "use web, web.ws

port = args[][0]
s = socket['ws://127.0.0.1:`port`/feed']
log[s.ok]
r = send[s, 'hello']
log[r.ok]
log[recv[s, 5]]
quiet = recv[s, 0.2]
log[quiet.reason]
r3 = send[s, 'slow']
early = recv[s, 0.2]
log[early.reason]
log[recv[s, 5]]
r2 = send[s, 'bye']
gone = recv[s, 5]
log[gone.reason]
log[gone.kind]
after = send[s, 'again']
log[after.reason]
close[s]
nobody = socket['ws://127.0.0.1:1/feed']
log[nobody.ok]
log[nobody.kind]
";

const SAID: &str = "true\ntrue\necho: hello\nno message in 0.2 s\nno message in 0.2 s\nslow and steady\nclosed\nsocket\nthe socket is closed\nfalse\nsocket\n";

#[test]
fn a_socket_talks_in_both_modes() {
    let port = echo_server();
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/crow_socket_app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.co"), PROGRAM).unwrap();
    for mode in [&["run", "-i"][..], &["run"][..]] {
        let out = Command::new(env!("CARGO_BIN_EXE_crow")).args(mode).arg("main.co").arg(port.to_string()).current_dir(&dir).output().unwrap();
        let said = String::from_utf8_lossy(&out.stdout);
        assert_eq!(said, SAID, "{:?} said:\n{}\nand on stderr:\n{}", mode, said, String::from_utf8_lossy(&out.stderr));
    }
}
