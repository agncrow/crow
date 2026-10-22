//! `crow kernel` from the outside, as Jupyter drives it: a connection file
//! names the sockets, each message is signed with its key, the cells run in
//! one session (a name a cell defines is the next cell's), what a cell logs
//! streams to iopub, its value is its result, and an error is the cell's
//! error reply.

use serde_json::{json, Value};
use std::time::Duration;
use zeromq::{Socket, SocketRecv, SocketSend};

const KEY: &str = "a-test-key";

fn sign(parts: &[String]) -> String {
    use hmac::{Hmac, Mac};
    let mut mac = <Hmac<sha2::Sha256> as Mac>::new_from_slice(KEY.as_bytes()).unwrap();
    for p in parts {
        mac.update(p.as_bytes());
    }
    mac.finalize().into_bytes().iter().map(|b| format!("{:02x}", b)).collect()
}

fn request(msg_type: &str, content: Value) -> (String, zeromq::ZmqMessage) {
    let id = format!("{}-{}", msg_type, std::process::id());
    let parts = vec![
        json!({"msg_id": id, "session": "s", "username": "test", "date": "2026-09-30T00:00:00Z", "msg_type": msg_type, "version": "5.3"}).to_string(),
        "{}".to_string(),
        "{}".to_string(),
        content.to_string(),
    ];
    let mut frames = vec![bytes::Bytes::from("<IDS|MSG>"), bytes::Bytes::from(sign(&parts))];
    frames.extend(parts.into_iter().map(bytes::Bytes::from));
    (id, zeromq::ZmqMessage::try_from(frames).unwrap())
}

/// A message's four parts, its signature checked.
fn parts(m: zeromq::ZmqMessage) -> (Value, Value, Value) {
    let frames: Vec<Vec<u8>> = m.into_vec().into_iter().map(|b| b.to_vec()).collect();
    let at = frames.iter().position(|f| f.as_slice() == b"<IDS|MSG>").expect("a delimiter");
    let text: Vec<String> = frames[at + 1..].iter().map(|f| String::from_utf8_lossy(f).to_string()).collect();
    assert_eq!(text[0], sign(&text[1..5]), "the kernel signs what it sends");
    let v = |s: &str| serde_json::from_str::<Value>(s).unwrap();
    (v(&text[1]), v(&text[2]), v(&text[4]))
}

/// What iopub broadcast for a request, up to its idle status.
async fn broadcasts(iopub: &mut zeromq::SubSocket, id: &str) -> Vec<(String, Value)> {
    let mut out = Vec::new();
    loop {
        let m = tokio::time::timeout(Duration::from_secs(60), iopub.recv()).await.expect("iopub answers").unwrap();
        let (header, parent, content) = parts(m);
        if parent.get("msg_id").and_then(Value::as_str) != Some(id) {
            continue;
        }
        let kind = header["msg_type"].as_str().unwrap_or("").to_string();
        let idle = kind == "status" && content["execution_state"] == "idle";
        out.push((kind, content));
        if idle {
            return out;
        }
    }
}

#[test]
fn a_notebook_runs_its_cells_in_one_session() {
    // five free ports, held until all are picked
    let held: Vec<std::net::TcpListener> = (0..5).map(|_| std::net::TcpListener::bind("127.0.0.1:0").unwrap()).collect();
    let ports: Vec<u16> = held.iter().map(|l| l.local_addr().unwrap().port()).collect();
    drop(held);
    let dir = std::env::temp_dir().join(format!("crow_kernel_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let file = dir.join("connection.json");
    std::fs::write(&file, json!({"transport": "tcp", "ip": "127.0.0.1", "shell_port": ports[0], "iopub_port": ports[1], "stdin_port": ports[2], "control_port": ports[3], "hb_port": ports[4], "key": KEY, "signature_scheme": "hmac-sha256", "kernel_name": "crow"}).to_string()).unwrap();
    let mut kernel = std::process::Command::new(env!("CARGO_BIN_EXE_crow")).arg("kernel").arg(&file).current_dir(&dir).spawn().unwrap();
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let mut shell = zeromq::DealerSocket::new();
        let mut iopub = zeromq::SubSocket::new();
        // the kernel is up once its sockets answer
        for _ in 0..100 {
            if shell.connect(&format!("tcp://127.0.0.1:{}", ports[0])).await.is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        iopub.connect(&format!("tcp://127.0.0.1:{}", ports[1])).await.unwrap();
        iopub.subscribe("").await.unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;

        let (_, info) = request("kernel_info_request", json!({}));
        shell.send(info).await.unwrap();
        let (header, _, content) = parts(tokio::time::timeout(Duration::from_secs(30), shell.recv()).await.expect("a reply").unwrap());
        assert_eq!(header["msg_type"], "kernel_info_reply");
        assert_eq!(content["language_info"]["name"], "crow");

        // a cell: what it logs streams, its value is its result
        let (id, cell) = request("execute_request", json!({"code": "x = 20\nlog['hello']\nx + 1", "silent": false}));
        shell.send(cell).await.unwrap();
        let said = broadcasts(&mut iopub, &id).await;
        let (_, _, reply) = parts(shell.recv().await.unwrap());
        assert_eq!(reply["status"], "ok", "{:?}", reply);
        assert_eq!(reply["execution_count"], 1);
        let streamed: String = said.iter().filter(|(k, _)| k == "stream").map(|(_, c)| c["text"].as_str().unwrap_or("").to_string()).collect();
        assert_eq!(streamed, "hello\n", "{:?}", said);
        let result = said.iter().find(|(k, _)| k == "execute_result").map(|(_, c)| c["data"]["text/plain"].clone());
        assert_eq!(result, Some(json!("21")), "{:?}", said);
        assert_eq!(said.first().map(|(k, c)| (k.as_str(), c["execution_state"].clone())), Some(("status", json!("busy"))));

        // the next cell sees what the first defined
        let (id, cell) = request("execute_request", json!({"code": "log[x * 2]", "silent": false}));
        shell.send(cell).await.unwrap();
        let said = broadcasts(&mut iopub, &id).await;
        let (_, _, reply) = parts(shell.recv().await.unwrap());
        assert_eq!(reply["execution_count"], 2);
        assert!(said.iter().any(|(k, c)| k == "stream" && c["text"] == "40\n"), "{:?}", said);

        // a function a cell defines is the next cell's to call (the IR check
        // had refused the call as one of a function the program does not
        // define, an internal error)
        let (id, cell) = request("execute_request", json!({"code": "fn double[n]\n    n * 2", "silent": false}));
        shell.send(cell).await.unwrap();
        let _ = broadcasts(&mut iopub, &id).await;
        let (_, _, reply) = parts(shell.recv().await.unwrap());
        assert_eq!(reply["status"], "ok", "{:?}", reply);
        let (id, cell) = request("execute_request", json!({"code": "double[x]", "silent": false}));
        shell.send(cell).await.unwrap();
        let said = broadcasts(&mut iopub, &id).await;
        let (_, _, reply) = parts(shell.recv().await.unwrap());
        assert_eq!(reply["status"], "ok", "{:?}", reply);
        assert!(said.iter().any(|(k, c)| k == "execute_result" && c["data"]["text/plain"] == "40"), "{:?}", said);

        // a frame's result is a table too
        let (id, cell) = request("execute_request", json!({"code": "use frame\nframe.new[a = [1, 2]]", "silent": false}));
        shell.send(cell).await.unwrap();
        let said = broadcasts(&mut iopub, &id).await;
        let _ = shell.recv().await.unwrap();
        let html = said.iter().find(|(k, _)| k == "execute_result").map(|(_, c)| c["data"]["text/html"].as_str().unwrap_or("").to_string()).unwrap_or_default();
        assert!(html.starts_with("<table><thead><tr><th>a</th></tr></thead>"), "{:?}", said);

        // an error is the cell's error, and the session goes on
        let (id, cell) = request("execute_request", json!({"code": "log[nope]", "silent": false}));
        shell.send(cell).await.unwrap();
        let said = broadcasts(&mut iopub, &id).await;
        let (_, _, reply) = parts(shell.recv().await.unwrap());
        assert_eq!(reply["status"], "error", "{:?}", reply);
        assert!(reply["evalue"].as_str().unwrap_or("").contains("nope"), "{:?}", reply);
        assert!(said.iter().any(|(k, _)| k == "error"), "{:?}", said);

        // an interrupt, on the control socket, stops a running cell at its
        // next loop turn, and the session goes on with what it had
        let mut control = zeromq::DealerSocket::new();
        control.connect(&format!("tcp://127.0.0.1:{}", ports[3])).await.unwrap();
        let (id, cell) = request("execute_request", json!({"code": "n = 0\nwhile true\n    n = n + 1", "silent": false}));
        shell.send(cell).await.unwrap();
        tokio::time::sleep(Duration::from_millis(500)).await;
        let (_, stop) = request("interrupt_request", json!({}));
        control.send(stop).await.unwrap();
        let (header, _, _) = parts(tokio::time::timeout(Duration::from_secs(30), control.recv()).await.expect("a reply").unwrap());
        assert_eq!(header["msg_type"], "interrupt_reply");
        let (_, _, reply) = parts(tokio::time::timeout(Duration::from_secs(30), shell.recv()).await.expect("the cell ends").unwrap());
        assert_eq!(reply["status"], "error", "{:?}", reply);
        assert_eq!(reply["ename"], "Interrupted", "{:?}", reply);
        let _ = broadcasts(&mut iopub, &id).await;
        let (id, cell) = request("execute_request", json!({"code": "n gt 0", "silent": false}));
        shell.send(cell).await.unwrap();
        let said = broadcasts(&mut iopub, &id).await;
        let _ = shell.recv().await.unwrap();
        assert!(said.iter().any(|(k, c)| k == "execute_result" && c["data"]["text/plain"] == "true"), "{:?}", said);

        let (_, bye) = request("shutdown_request", json!({"restart": false}));
        shell.send(bye).await.unwrap();
        let (header, _, _) = parts(tokio::time::timeout(Duration::from_secs(30), shell.recv()).await.expect("a reply").unwrap());
        assert_eq!(header["msg_type"], "shutdown_reply");
    });
    let status = kernel.wait().unwrap();
    assert!(status.success(), "{:?}", status);
    let _ = std::fs::remove_dir_all(&dir);
}
