//! The Model Context Protocol in both modes (spec, section 13: mcp): a
//! Crow client driven against a stand-in server speaking the 2026-07-28
//! revision — `server/discover`, `tools/list`, `tools/call` (a result, one
//! marked an error, an unknown tool), a response streamed as events, a
//! server that rejects a request's headers, one nobody answers — and a
//! Crow program serving `/mcp` driven by a stand-in client: discovery, the
//! tools listed from the functions themselves, a call answered, a wrong
//! argument and a tool's failure marked errors, an unknown tool, an unknown
//! method, a notification, a body that is no JSON, a request missing its
//! headers or `_meta`, and one of another protocol version — the same
//! lines from `crow run -i` and from the compiled program. And the stdio
//! transport: a Crow client against a Crow program serving over its
//! standard streams (`listen`), the server started as a command by the
//! client, interpreted and compiled on each side.

use serde_json::{json, Value};
use std::io::{Read, Write};
use std::process::Command;

fn respond(stream: &mut std::net::TcpStream, status: &str, content_type: &str, body: &str) {
    let _ = write!(stream, "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", status, content_type, body.len());
    let _ = stream.write_all(body.as_bytes());
}

/// A request read whole: its head, its path, its headers (lower-cased
/// names) and its body.
fn read_request(stream: &mut std::net::TcpStream) -> (String, Vec<(String, String)>, String) {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 65536];
    loop {
        let n = match stream.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        buf.extend_from_slice(&chunk[..n]);
        if let Some(head_end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
            let length: usize = head.lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse().unwrap_or(0))).unwrap_or(0);
            if buf.len() >= head_end + 4 + length {
                break;
            }
        }
    }
    let head_end = buf.windows(4).position(|w| w == b"\r\n\r\n").unwrap_or(buf.len());
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let body = String::from_utf8_lossy(&buf[(head_end + 4).min(buf.len())..]).to_string();
    let path = head.lines().next().unwrap_or("").split_whitespace().nth(1).unwrap_or("").to_string();
    let headers: Vec<(String, String)> = head.lines().skip(1).filter_map(|l| l.split_once(':').map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))).collect();
    (path, headers, body)
}

/// The stand-in server: every request answered by its method, as the
/// revision says — `tools/list` as a stream of events (a progress
/// notification, then the response), the rest as JSON; a request without
/// `Mcp-Method` rejected with a header mismatch.
fn answer(mut stream: std::net::TcpStream) {
    let (path, headers, body) = read_request(&mut stream);
    let header = |name: &str| headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone()).unwrap_or_default();
    if path != "/mcp" {
        return respond(&mut stream, "404 Not Found", "application/json", r#"{"jsonrpc": "2.0", "id": null, "error": {"code": -32601, "message": "no such endpoint"}}"#);
    }
    let sent: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    let id = sent["id"].clone();
    let method = sent["method"].as_str().unwrap_or("").to_string();
    if header("mcp-method") != method || header("mcp-protocol-version") != "2026-07-28" {
        let err = json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32020, "message": "Header mismatch"}});
        return respond(&mut stream, "400 Bad Request", "application/json", &err.to_string());
    }
    let meta = &sent["params"]["_meta"];
    if meta["io.modelcontextprotocol/protocolVersion"] != "2026-07-28" || meta["io.modelcontextprotocol/clientCapabilities"].is_null() {
        let err = json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32602, "message": "Invalid params: _meta"}});
        return respond(&mut stream, "400 Bad Request", "application/json", &err.to_string());
    }
    let server_info = json!({"io.modelcontextprotocol/serverInfo": {"name": "stand-in", "version": "1.0"}});
    match method.as_str() {
        "server/discover" => {
            let result = json!({"jsonrpc": "2.0", "id": id, "result": {"resultType": "complete", "supportedVersions": ["2026-07-28"], "capabilities": {"tools": {}}, "_meta": server_info, "instructions": "echo things", "ttlMs": 3600000, "cacheScope": "public"}});
            respond(&mut stream, "200 OK", "application/json", &result.to_string())
        }
        "tools/list" => {
            let tools = json!([
                {"name": "echo", "description": "says its words back", "inputSchema": {"type": "object", "properties": {"words": {"type": "string"}}, "required": ["words"]}},
                {"name": "fail", "description": "always fails", "inputSchema": {"type": "object", "additionalProperties": false}}
            ]);
            let progress = json!({"jsonrpc": "2.0", "method": "notifications/progress", "params": {"progressToken": "t", "progress": 1}});
            let result = json!({"jsonrpc": "2.0", "id": id, "result": {"resultType": "complete", "tools": tools, "_meta": server_info, "ttlMs": 60000, "cacheScope": "public"}});
            let events = format!("event: message\ndata: {}\n\n: keep-alive\n\ndata: {}\n\n", progress, result);
            respond(&mut stream, "200 OK", "text/event-stream", &events)
        }
        "tools/call" => {
            let name = sent["params"]["name"].as_str().unwrap_or("");
            if header("mcp-name") != name {
                let err = json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32020, "message": "Header mismatch: Mcp-Name"}});
                return respond(&mut stream, "400 Bad Request", "application/json", &err.to_string());
            }
            let result = match name {
                "echo" => json!({"resultType": "complete", "content": [{"type": "text", "text": format!("echo: {}", sent["params"]["arguments"]["words"].as_str().unwrap_or(""))}], "isError": false, "_meta": server_info}),
                "fail" => json!({"resultType": "complete", "content": [{"type": "text", "text": "it failed, as it always does"}], "isError": true, "_meta": server_info}),
                other => {
                    let err = json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32602, "message": format!("Unknown tool: {}", other)}});
                    return respond(&mut stream, "200 OK", "application/json", &err.to_string());
                }
            };
            respond(&mut stream, "200 OK", "application/json", &json!({"jsonrpc": "2.0", "id": id, "result": result}).to_string())
        }
        _ => {
            let err = json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "Method not found"}});
            respond(&mut stream, "404 Not Found", "application/json", &err.to_string())
        }
    }
}

fn stand_in_server() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            std::thread::spawn(move || answer(stream));
        }
    });
    port
}

const CLIENT: &str = "use mcp

port = args[][0]
s = server['http://127.0.0.1:`port`/mcp']
log[s.url]
d = discover[s]
log[d.supportedVersions[0]]
log[d.capabilities.tools.ok]
log[d.instructions]
log[d['_meta']['io.modelcontextprotocol/serverInfo'].name]
ts = catalog[s]
log[len[ts]]
log[ts[0].name]
log[ts[0].description]
log[ts[0].inputSchema.required[0]]
log[tool[s, 'echo', map[words = 'hi there']]]
log[tool[s, 'echo', json['{\"words\": \"from a parsed value\"}']]]
failed = tool[s, 'fail', map[]]
log[failed.ok]
log[failed.kind]
log[failed.reason]
unknown = tool[s, 'nothing', map[]]
log[unknown.ok]
log[unknown.reason]
nowhere = server['http://127.0.0.1:1/mcp']
gone = catalog[nowhere]
log[gone.ok]
log[gone.kind]
log[server['localhost:8000/mcp'].reason]
";

#[test]
fn a_program_reaches_an_mcp_server_in_both_modes() {
    let port = stand_in_server();
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/crow_mcp_client");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.co"), CLIENT).unwrap();
    let said = format!(
        "http://127.0.0.1:{port}/mcp\n\
2026-07-28\n\
true\n\
echo things\n\
stand-in\n\
2\n\
echo\n\
says its words back\n\
words\n\
echo: hi there\n\
echo: from a parsed value\n\
false\n\
mcp\n\
it failed, as it always does\n\
false\n\
the MCP server answered an error -32602: Unknown tool: nothing\n\
false\n\
mcp\n\
an MCP server is reached at its endpoint's URL: localhost:8000/mcp is none — http://localhost:8000/mcp — or started as a command: server[command = 'npx -y @modelcontextprotocol/server-everything']\n"
    );
    for mode in [&["run", "-i"][..], &["run"][..]] {
        let out = Command::new(env!("CARGO_BIN_EXE_crow")).args(mode).arg("main.co").arg(port.to_string()).current_dir(&dir).env("CROW_BUILD_PROFILE", "dev").output().unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), said, "{:?}: {}", mode, String::from_utf8_lossy(&out.stderr));
    }
}

/// A Crow program as an MCP server: three tools — one of which fails for one
/// argument, and one taking an object, typed by the fields its body reads
/// (spec, Tools are the functions) — answered over `/mcp`.
const SERVER: &str = "use web, mcp

--- the current weather in a city
fn weather[city]
    if city eq 'Nowhere'
        return error['no station in `city`', 'weather']
    return 'sunny in `upper[city]`'

--- the forecast for a city, for a number of days
fn forecast[city, days = 2]
    return '`days` days of sun in `upper[city]`'

--- ships an order: its id, and its lines, each a sku and a count
fn ship[order]
    lines = 0
    for line in order.lines
        lines = lines + 1
    return 'shipped order `order.id` with `lines` lines'

fn served[req]
    return answer[req, tools = [weather, forecast, ship], name = 'weather-server', instructions = 'ask about the weather']

route['/mcp', 'POST', served]
serve[PORT]
";

/// A stand-in client: one POST, with the headers the revision requires
/// unless told to leave them out; the status line and the body.
fn post(port: u16, body: &str, headers: &[(&str, &str)]) -> (String, String) {
    let mut http = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    let extra: String = headers.iter().map(|(k, v)| format!("{}: {}\r\n", k, v)).collect();
    write!(http, "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\n{}Content-Length: {}\r\nConnection: close\r\n\r\n", port, extra, body.len()).unwrap();
    http.write_all(body.as_bytes()).unwrap();
    let mut raw = String::new();
    let _ = http.read_to_string(&mut raw);
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((raw.as_str(), ""));
    let status = head.lines().next().unwrap_or("").to_string();
    (status, body.to_string())
}

fn modern(method: &str, params: Value, id: Value) -> String {
    let mut params = params;
    params["_meta"] = json!({"io.modelcontextprotocol/protocolVersion": "2026-07-28", "io.modelcontextprotocol/clientInfo": {"name": "stand-in", "version": "1.0"}, "io.modelcontextprotocol/clientCapabilities": {}});
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string()
}

/// A served program's answers, each the status and the body read back as
/// JSON (or the text where there is none), for one mode.
fn served_answers(mode: &[&str], port: u16) -> Vec<String> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("target/crow_mcp_server_{}", if mode.len() == 2 { "i" } else { "c" }));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.co"), SERVER.replace("PORT", &port.to_string())).unwrap();
    // (its own process group: the compiled program is a process crow
    // starts, and killing crow alone had left it listening)
    use std::os::unix::process::CommandExt as _;
    let mut child = Command::new(env!("CARGO_BIN_EXE_crow")).args(mode).arg("main.co").current_dir(&dir).env("CROW_BUILD_PROFILE", "dev").stdout(std::process::Stdio::null()).stderr(std::process::Stdio::piped()).process_group(0).spawn().unwrap();
    // (the compiled program builds first: wait for the port)
    let started = std::time::Instant::now();
    loop {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            break;
        }
        if let Ok(Some(status)) = child.try_wait() {
            let mut err = String::new();
            if let Some(e) = child.stderr.as_mut() {
                let _ = e.read_to_string(&mut err);
            }
            panic!("{:?} ended before serving: {} {}", mode, status, err);
        }
        assert!(started.elapsed() < std::time::Duration::from_secs(600), "{:?}: the server did not come up", mode);
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    let version = ("MCP-Protocol-Version", "2026-07-28");
    let steps: Vec<(String, Vec<(&str, &str)>)> = vec![
        (modern("server/discover", json!({}), json!(1)), vec![version, ("Mcp-Method", "server/discover")]),
        (modern("tools/list", json!({}), json!(2)), vec![version, ("Mcp-Method", "tools/list")]),
        (modern("tools/call", json!({"name": "weather", "arguments": {"city": "Lisbon"}}), json!(3)), vec![version, ("Mcp-Method", "tools/call"), ("Mcp-Name", "weather")]),
        (modern("tools/call", json!({"name": "forecast", "arguments": {"city": "Porto", "days": "many"}}), json!(4)), vec![version, ("Mcp-Method", "tools/call"), ("Mcp-Name", "forecast")]),
        (modern("tools/call", json!({"name": "weather", "arguments": {"city": "Nowhere"}}), json!(5)), vec![version, ("Mcp-Method", "tools/call"), ("Mcp-Name", "weather")]),
        (modern("tools/call", json!({"name": "nothing", "arguments": {}}), json!("six")), vec![version, ("Mcp-Method", "tools/call"), ("Mcp-Name", "nothing")]),
        (modern("resources/list", json!({}), json!(7)), vec![version, ("Mcp-Method", "resources/list")]),
        (json!({"jsonrpc": "2.0", "method": "notifications/cancelled", "params": {"requestId": 3}}).to_string(), vec![version, ("Mcp-Method", "notifications/cancelled")]),
        ("this is not json".to_string(), vec![version, ("Mcp-Method", "tools/list")]),
        (modern("tools/list", json!({}), json!(9)), vec![]),
        (json!({"jsonrpc": "2.0", "id": 10, "method": "tools/list", "params": {}}).to_string(), vec![version, ("Mcp-Method", "tools/list")]),
        (modern("tools/list", json!({}), json!(11)).replace("2026-07-28", "2025-11-25"), vec![("MCP-Protocol-Version", "2025-11-25"), ("Mcp-Method", "tools/list")]),
        (modern("tools/call", json!({"name": "weather", "arguments": {"city": "Lisbon"}}), json!(12)), vec![version, ("Mcp-Method", "tools/call"), ("Mcp-Name", "forecast")]),
        (modern("tools/call", json!({"name": "ship", "arguments": {"order": {"id": 7, "lines": [{"sku": "A1", "count": 2}, {"sku": "B2", "count": 1}]}}}), json!(13)), vec![version, ("Mcp-Method", "tools/call"), ("Mcp-Name", "ship")]),
    ];
    let mut answers = Vec::new();
    for (body, headers) in &steps {
        let (status, body) = post(port, body, headers);
        let shown = match serde_json::from_str::<Value>(&body) {
            Ok(v) => serde_json::to_string(&v).unwrap(),
            Err(_) => format!("{:?}", body),
        };
        answers.push(format!("{} | {}", status, shown));
    }
    // the whole group — crow and the program it started — then the port
    let _ = Command::new("kill").args(["-TERM", "--", &format!("-{}", child.id())]).status();
    let _ = child.kill();
    let _ = child.wait();
    let gone = std::time::Instant::now();
    while std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() && gone.elapsed() < std::time::Duration::from_secs(10) {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    answers
}

#[test]
fn a_program_serves_mcp_in_both_modes() {
    let expected = vec![
        r#"HTTP/1.1 200 OK | {"id":1,"jsonrpc":"2.0","result":{"_meta":{"io.modelcontextprotocol/serverInfo":{"name":"weather-server","version":"0.7.0"}},"cacheScope":"public","capabilities":{"tools":{}},"instructions":"ask about the weather","resultType":"complete","supportedVersions":["2026-07-28"],"ttlMs":3600000}}"#,
        r#"HTTP/1.1 200 OK | {"id":2,"jsonrpc":"2.0","result":{"_meta":{"io.modelcontextprotocol/serverInfo":{"name":"weather-server","version":"0.7.0"}},"cacheScope":"public","resultType":"complete","tools":[{"description":"the current weather in a city","inputSchema":{"properties":{"city":{"type":"string"}},"required":["city"],"type":"object"},"name":"weather"},{"description":"the forecast for a city, for a number of days","inputSchema":{"properties":{"city":{"type":"string"},"days":{"type":"integer"}},"required":["city","days"],"type":"object"},"name":"forecast"},{"description":"ships an order: its id, and its lines, each a sku and a count","inputSchema":{"properties":{"order":{"type":"object"}},"required":["order"],"type":"object"},"name":"ship"}],"ttlMs":60000}}"#,
        r#"HTTP/1.1 200 OK | {"id":3,"jsonrpc":"2.0","result":{"_meta":{"io.modelcontextprotocol/serverInfo":{"name":"weather-server","version":"0.7.0"}},"content":[{"text":"sunny in LISBON","type":"text"}],"isError":false,"resultType":"complete"}}"#,
        r#"HTTP/1.1 200 OK | {"id":4,"jsonrpc":"2.0","result":{"_meta":{"io.modelcontextprotocol/serverInfo":{"name":"weather-server","version":"0.7.0"}},"content":[{"text":"a parsed value holds text 'many' where the program reads an int (forecast[]'s 'days')","type":"text"}],"isError":true,"resultType":"complete"}}"#,
        r#"HTTP/1.1 200 OK | {"id":5,"jsonrpc":"2.0","result":{"_meta":{"io.modelcontextprotocol/serverInfo":{"name":"weather-server","version":"0.7.0"}},"content":[{"text":"no station in Nowhere","type":"text"}],"isError":true,"resultType":"complete"}}"#,
        r#"HTTP/1.1 200 OK | {"error":{"code":-32602,"message":"Unknown tool: nothing"},"id":"six","jsonrpc":"2.0"}"#,
        r#"HTTP/1.1 404 Not Found | {"error":{"code":-32601,"message":"Method not found: resources/list"},"id":7,"jsonrpc":"2.0"}"#,
        r#"HTTP/1.1 202 Accepted | """#,
        r#"HTTP/1.1 400 Bad Request | {"error":{"code":-32700,"message":"the body is no JSON"},"id":null,"jsonrpc":"2.0"}"#,
        r#"HTTP/1.1 400 Bad Request | {"error":{"code":-32020,"message":"Header mismatch: MCP-Protocol-Version and Mcp-Method must match the body"},"id":9,"jsonrpc":"2.0"}"#,
        r#"HTTP/1.1 400 Bad Request | {"error":{"code":-32602,"message":"a request carries its protocol version and the client's capabilities in _meta"},"id":10,"jsonrpc":"2.0"}"#,
        r#"HTTP/1.1 400 Bad Request | {"error":{"code":-32022,"data":{"requested":"2025-11-25","supported":["2026-07-28"]},"message":"Unsupported protocol version"},"id":11,"jsonrpc":"2.0"}"#,
        r#"HTTP/1.1 400 Bad Request | {"error":{"code":-32020,"message":"Header mismatch: Mcp-Name must match the tool called"},"id":12,"jsonrpc":"2.0"}"#,
        r#"HTTP/1.1 200 OK | {"id":13,"jsonrpc":"2.0","result":{"_meta":{"io.modelcontextprotocol/serverInfo":{"name":"weather-server","version":"0.7.0"}},"content":[{"text":"shipped order 7 with 2 lines","type":"text"}],"isError":false,"resultType":"complete"}}"#,
    ];
    for (mode, port) in [(&["run", "-i"][..], 3495u16), (&["run"][..], 3496u16)] {
        let answers = served_answers(mode, port);
        assert_eq!(answers, expected, "{:?}", mode);
    }
}

/// The same three tools served over the program's standard streams: a
/// client starts it as a command and talks to it line by line.
const SERVER_STDIO: &str = "use mcp

--- the current weather in a city
fn weather[city]
    if city eq 'Nowhere'
        return error['no station in `city`', 'weather']
    return 'sunny in `upper[city]`'

--- the forecast for a city, for a number of days
fn forecast[city, days = 2]
    return '`days` days of sun in `upper[city]`'

--- ships an order: its id, and its lines, each a sku and a count
fn ship[order]
    lines = 0
    for line in order.lines
        lines = lines + 1
    return 'shipped order `order.id` with `lines` lines'

listen[tools = [weather, forecast, ship], name = 'weather-server', instructions = 'ask about the weather']
";

/// A client over stdio: the server's command is its first argument, and
/// the calls are the HTTP client's — discovery, the list, a call with a
/// map and with a parsed value, a wrong argument, a tool's failure, an
/// unknown tool, an object handed to a tool.
const CLIENT_STDIO: &str = "use mcp

s = server[command = args[][0]]
log[s.url]
log[len[s.command] gt 0]
d = discover[s]
log[d.supportedVersions[0]]
log[d.instructions]
log[d['_meta']['io.modelcontextprotocol/serverInfo'].name]
ts = catalog[s]
log[len[ts]]
log[ts[0].name]
log[ts[0].description]
log[ts[2].inputSchema.properties.order.type]
log[tool[s, 'weather', map[city = 'Lisbon']]]
log[tool[s, 'forecast', json['{\"city\": \"Porto\", \"days\": 3}']]]
bad = tool[s, 'forecast', json['{\"city\": \"Porto\", \"days\": \"many\"}']]
log[bad.ok]
log[bad.kind]
log[bad.reason]
failed = tool[s, 'weather', map[city = 'Nowhere']]
log[failed.ok]
log[failed.reason]
unknown = tool[s, 'nothing', map[]]
log[unknown.reason]
log[tool[s, 'ship', json['{\"order\": {\"id\": 7, \"lines\": [{\"sku\": \"A1\", \"count\": 2}, {\"sku\": \"B2\", \"count\": 1}]}}']]]
";

#[test]
fn a_program_reaches_an_mcp_server_over_stdio_in_both_modes() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/crow_mcp_stdio");
    // (the server in a directory of its own: a `.crow_build` is one
    // program's, and the client's compiled run has its own)
    let server_dir = root.join("server");
    std::fs::create_dir_all(&server_dir).unwrap();
    std::fs::write(server_dir.join("server.co"), SERVER_STDIO).unwrap();
    let said = "\n\
true\n\
2026-07-28\n\
ask about the weather\n\
weather-server\n\
3\n\
weather\n\
the current weather in a city\n\
object\n\
sunny in LISBON\n\
3 days of sun in PORTO\n\
false\n\
mcp\n\
a parsed value holds text 'many' where the program reads an int (forecast[]'s 'days')\n\
false\n\
no station in Nowhere\n\
the MCP server answered an error -32602: Unknown tool: nothing\n\
shipped order 7 with 2 lines\n";
    for server_mode in ["run -i", "run"] {
        // the server as a command the client starts: into its directory,
        // then crow interpreting or compiling it (the build's own output
        // goes to standard error, as the protocol wants)
        let command = format!("cd '{}' && '{}' {} server.co", server_dir.display(), env!("CARGO_BIN_EXE_crow"), server_mode);
        for client_mode in [&["run", "-i"][..], &["run"][..]] {
            let dir = root.join(format!("client_{}", if client_mode.len() == 2 { "i" } else { "c" }));
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("main.co"), CLIENT_STDIO).unwrap();
            let out = Command::new(env!("CARGO_BIN_EXE_crow")).args(client_mode).arg("main.co").arg(&command).current_dir(&dir).env("CROW_BUILD_PROFILE", "dev").output().unwrap();
            assert_eq!(String::from_utf8_lossy(&out.stdout), said, "client {:?}, server {}: {}", client_mode, server_mode, String::from_utf8_lossy(&out.stderr));
        }
    }
}
