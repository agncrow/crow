//! A language model as a connection, in both modes (spec, section 13:
//! llm): a server speaking both wire formats — the OpenAI-compatible chat
//! completions and embeddings, and Anthropic's messages, each plain and
//! streamed as server-sent events over chunked transfer encoding, with
//! events straddling the chunks — and a program driven against it from
//! `crow run -i` and from the compiled program: a reply, a tool call the
//! compiler wrote the schema for, the conversation continued with the
//! tool's result, a reply followed piece by piece, a streamed tool call
//! gathered, embeddings, and the refusals (a bad key, a body that is no
//! JSON, a server nobody answers), the same lines from both.

use std::io::{Read, Write};
use std::process::Command;

fn chunked(pieces: &[String]) -> Vec<u8> {
    let mut out = Vec::new();
    for p in pieces {
        out.extend_from_slice(format!("{:x}\r\n", p.len()).as_bytes());
        out.extend_from_slice(p.as_bytes());
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(b"0\r\n\r\n");
    out
}

/// The whole stream cut into pieces of an odd length, so an event
/// straddles chunk boundaries (and a multi-byte character may too).
fn straddled(events: &str, width: usize) -> Vec<String> {
    let bytes = events.as_bytes();
    let mut pieces = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let end = (at + width).min(bytes.len());
        pieces.push(String::from_utf8_lossy(&bytes[at..end]).to_string());
        at = end;
    }
    // (a cut inside a character would lose bytes through the lossy read:
    // cut on character boundaries instead, the widths uneven)
    let mut out = Vec::new();
    let mut rest = events;
    let mut w = width;
    while !rest.is_empty() {
        let mut cut = w.min(rest.len());
        while !rest.is_char_boundary(cut) {
            cut -= 1;
        }
        out.push(rest[..cut].to_string());
        rest = &rest[cut..];
        w = if w == width { width + 3 } else { width };
    }
    drop(pieces);
    out
}

fn respond(mut stream: std::net::TcpStream, status: &str, body: &str) {
    let _ = write!(stream, "HTTP/1.1 {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", status, body.len());
    let _ = stream.write_all(body.as_bytes());
}

fn respond_stream(mut stream: std::net::TcpStream, events: &str, width: usize) {
    let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n");
    let _ = stream.write_all(&chunked(&straddled(events, width)));
}

fn openai_stream(tool: bool) -> String {
    let lines: Vec<String> = if tool {
        vec![
            r#"{"choices": [{"delta": {"role": "assistant", "content": null, "tool_calls": [{"index": 0, "id": "call_1", "type": "function", "function": {"name": "weather", "arguments": ""}}]}, "finish_reason": null}]}"#.into(),
            r#"{"choices": [{"delta": {"tool_calls": [{"index": 0, "function": {"arguments": "{\"ci"}}]}, "finish_reason": null}]}"#.into(),
            r#"{"choices": [{"delta": {"tool_calls": [{"index": 0, "function": {"arguments": "ty\": \"Lisbon\"}"}}]}, "finish_reason": null}]}"#.into(),
            r#"{"choices": [{"delta": {}, "finish_reason": "tool_calls"}]}"#.into(),
            r#"{"choices": [], "usage": {"prompt_tokens": 7, "completion_tokens": 5}}"#.into(),
        ]
    } else {
        vec![
            r#"{"choices": [{"delta": {"role": "assistant", "content": ""}, "finish_reason": null}]}"#.into(),
            r#"{"choices": [{"delta": {"content": "Ulaan"}, "finish_reason": null}]}"#.into(),
            r#"{"choices": [{"delta": {"content": "baatar"}, "finish_reason": null}]}"#.into(),
            r#"{"choices": [{"delta": {"content": " é!"}, "finish_reason": null}]}"#.into(),
            r#"{"choices": [{"delta": {}, "finish_reason": "stop"}]}"#.into(),
            r#"{"choices": [], "usage": {"prompt_tokens": 9, "completion_tokens": 4}}"#.into(),
        ]
    };
    let mut out = String::new();
    for (i, l) in lines.iter().enumerate() {
        out.push_str(&format!("data: {}\n\n", l));
        if i == 1 {
            out.push_str(": keep-alive\n\n");
        }
    }
    out.push_str("data: [DONE]\n\n");
    out
}

fn anthropic_stream(tool: bool) -> String {
    let events: Vec<(&str, String)> = if tool {
        vec![
            ("message_start", r#"{"type": "message_start", "message": {"usage": {"input_tokens": 12, "output_tokens": 1}}}"#.into()),
            ("content_block_start", r#"{"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}"#.into()),
            ("content_block_delta", r#"{"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "Let me check."}}"#.into()),
            ("content_block_stop", r#"{"type": "content_block_stop", "index": 0}"#.into()),
            ("content_block_start", r#"{"type": "content_block_start", "index": 1, "content_block": {"type": "tool_use", "id": "toolu_1", "name": "weather", "input": {}}}"#.into()),
            ("content_block_delta", r#"{"type": "content_block_delta", "index": 1, "delta": {"type": "input_json_delta", "partial_json": "{\"city\": "}}"#.into()),
            ("content_block_delta", r#"{"type": "content_block_delta", "index": 1, "delta": {"type": "input_json_delta", "partial_json": "\"Lisbon\"}"}}"#.into()),
            ("content_block_stop", r#"{"type": "content_block_stop", "index": 1}"#.into()),
            ("message_delta", r#"{"type": "message_delta", "delta": {"stop_reason": "tool_use"}, "usage": {"output_tokens": 15}}"#.into()),
            ("message_stop", r#"{"type": "message_stop"}"#.into()),
        ]
    } else {
        vec![
            ("message_start", r#"{"type": "message_start", "message": {"usage": {"input_tokens": 12, "output_tokens": 1}}}"#.into()),
            ("content_block_start", r#"{"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}"#.into()),
            ("ping", r#"{"type": "ping"}"#.into()),
            ("content_block_delta", r#"{"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "Ulaan"}}"#.into()),
            ("content_block_delta", r#"{"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "baatar é!"}}"#.into()),
            ("content_block_stop", r#"{"type": "content_block_stop", "index": 0}"#.into()),
            ("message_delta", r#"{"type": "message_delta", "delta": {"stop_reason": "end_turn"}, "usage": {"output_tokens": 4}}"#.into()),
            ("message_stop", r#"{"type": "message_stop"}"#.into()),
        ]
    };
    events.iter().map(|(e, d)| format!("event: {}\ndata: {}\n\n", e, d)).collect()
}

/// One request answered by what it asked: the path names the format, the
/// body says whether a reply is streamed and whether tools were offered
/// (a tool call comes once, then its result is read back).
fn answer(mut stream: std::net::TcpStream) {
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
    let header = |name: &str| head.lines().find_map(|l| l.split_once(':').filter(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.trim().to_string())).unwrap_or_default();
    let streamed = body.contains("\"stream\": true");
    let offered = body.contains("\"tools\": [");
    let told = body.contains("\"role\": \"tool\"") || body.contains("\"type\": \"tool_result\"");
    let wants_tool = offered && !told;
    if path.starts_with("/broken") {
        return respond(stream, "200 OK", "this is not json");
    }
    match path.as_str() {
        "/v1/chat/completions" => {
            if header("authorization") == "Bearer bad" {
                return respond(stream, "401 Unauthorized", r#"{"error": {"message": "Incorrect API key provided: bad", "type": "invalid_request_error"}}"#);
            }
            if streamed {
                return respond_stream(stream, &openai_stream(wants_tool), 23);
            }
            if wants_tool {
                return respond(stream, "200 OK", r#"{"choices": [{"message": {"role": "assistant", "content": null, "tool_calls": [{"id": "call_1", "type": "function", "function": {"name": "weather", "arguments": "{\"city\": \"Lisbon\"}"}}]}, "finish_reason": "tool_calls"}], "usage": {"prompt_tokens": 7, "completion_tokens": 5}}"#);
            }
            let text = if told { "It is sunny in LISBON." } else { "Ulaanbaatar" };
            respond(stream, "200 OK", &format!(r#"{{"choices": [{{"message": {{"role": "assistant", "content": "{}"}}, "finish_reason": "stop"}}], "usage": {{"prompt_tokens": 21, "completion_tokens": 3}}}}"#, text))
        }
        "/v1/embeddings" => {
            // (a row per text in the input list)
            let inputs = body.split("\"input\": [").nth(1).and_then(|rest| rest.split(']').next()).unwrap_or("");
            let n = inputs.matches('"').count() / 2;
            let rows: Vec<String> = (0..n).map(|i| format!(r#"{{"embedding": [{}.0, 0.5, -0.25]}}"#, i)).collect();
            respond(stream, "200 OK", &format!(r#"{{"data": [{}], "usage": {{"prompt_tokens": 3}}}}"#, rows.join(", ")))
        }
        "/v1/messages" => {
            if header("x-api-key") == "bad" {
                return respond(stream, "401 Unauthorized", r#"{"type": "error", "error": {"type": "authentication_error", "message": "invalid x-api-key"}}"#);
            }
            if streamed {
                return respond_stream(stream, &anthropic_stream(wants_tool), 31);
            }
            if wants_tool {
                return respond(stream, "200 OK", r#"{"content": [{"type": "text", "text": "Let me check."}, {"type": "tool_use", "id": "toolu_1", "name": "weather", "input": {"city": "Lisbon"}}], "stop_reason": "tool_use", "usage": {"input_tokens": 12, "output_tokens": 15}}"#);
            }
            let text = if told { "It is sunny." } else { "Ulaanbaatar" };
            respond(stream, "200 OK", &format!(r#"{{"content": [{{"type": "text", "text": "{}"}}], "stop_reason": "end_turn", "usage": {{"input_tokens": 21, "output_tokens": 3}}}}"#, text))
        }
        _ => respond(stream, "404 Not Found", r#"{"error": {"message": "no such path"}}"#),
    }
}

fn model_server() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            std::thread::spawn(move || answer(stream));
        }
    });
    port
}

const PROGRAM: &str = "use llm

port = args[][0]
m = reach['openai://127.0.0.1:`port`/test-model']
log[m.provider]
log[m.model]
log[m.base]
talk = ask['What is the capital of Mongolia?']
r = chat[m, talk, system = 'one word', max = 32]
log[r.text]
log[r.stop]
log[r.usage.input]
log[r.usage.output]
log[len[r.calls]]

--- the current weather in a city
fn weather[city]
    return 'sunny in `upper[city]`'

r2 = chat[m, talk, tools = [weather]]
log[r2.stop]
log[r2.calls[0].name]
log[r2.calls[0].args.city]
talk2 = said[talk, r2]
talk2 = told[talk2, r2.calls[0], weather[r2.calls[0].args.city]]
log[len[talk2]]
r3 = chat[m, talk2, tools = [weather]]
log[r3.text]

s = follow[m, talk]
piece = next[s]
while piece.ok and len[piece] gt 0
    log['piece: `piece`']
    piece = next[s]
whole = reply[s]
log[whole.text]
log[whole.stop]
log[whole.usage.output]

s2 = follow[m, talk, tools = [weather]]
piece = next[s2]
while piece.ok and len[piece] gt 0
    log['piece: `piece`']
    piece = next[s2]
whole2 = reply[s2]
log[whole2.stop]
log[whole2.calls[0].args.city]

e = embed[m, ['one', 'two']]
log[e]

a = reach['anthropic://claude-test']
log[a.base]
ra = chat[a, talk]
log[ra.text]
log[ra.usage.input]
ra2 = chat[a, talk, tools = [weather]]
log[ra2.text]
log[ra2.stop]
log[ra2.calls[0].id]
log[ra2.calls[0].args.city]
talka = told[said[talk, ra2], ra2.calls[0], 'sunny']
ra3 = chat[a, talka, tools = [weather]]
log[ra3.text]
sa = follow[a, talk]
piece = next[sa]
while piece.ok and len[piece] gt 0
    log['piece: `piece`']
    piece = next[sa]
log[reply[sa].text]
log[reply[sa].stop]
sa2 = follow[a, talk, tools = [weather]]
piece = next[sa2]
while piece.ok and len[piece] gt 0
    log['piece: `piece`']
    piece = next[sa2]
wa = reply[sa2]
log[wa.stop]
log[wa.calls[0].name]
log[wa.calls[0].args.city]
log[wa.usage.output]

bad = reach['openai://127.0.0.1:`port`/test-model?key=CROW_LLM_TEST_BAD_KEY']
rb = chat[bad, talk]
log[rb.ok]
log[rb.reason]
broken = reach['openai://127.0.0.1:`port`/test-model?path=/broken']
rk = chat[broken, talk]
log[rk.reason]
nobody = reach['openai://127.0.0.1:1/test-model']
rn = chat[nobody, talk]
log[rn.ok]
log[rn.kind]
sn = follow[nobody, talk]
log[sn.ok]
log[sn.kind]
";

#[test]
fn a_program_talks_to_a_model_in_both_modes() {
    let port = model_server();
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/crow_llm_server_app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.co"), PROGRAM).unwrap();
    let said = format!(
        "openai\n\
test-model\n\
http://127.0.0.1:{port}\n\
Ulaanbaatar\n\
end\n\
21\n\
3\n\
0\n\
tool\n\
weather\n\
Lisbon\n\
3\n\
It is sunny in LISBON.\n\
piece: Ulaan\n\
piece: baatar\n\
piece:  é!\n\
Ulaanbaatar é!\n\
end\n\
4\n\
tool\n\
Lisbon\n\
[[0, 0.5, -0.25], [1, 0.5, -0.25]]\n\
http://127.0.0.1:{port}\n\
Ulaanbaatar\n\
21\n\
Let me check.\n\
tool\n\
toolu_1\n\
Lisbon\n\
It is sunny.\n\
piece: Ulaan\n\
piece: baatar é!\n\
Ulaanbaatar é!\n\
end\n\
piece: Let me check.\n\
tool\n\
weather\n\
Lisbon\n\
15\n\
false\n\
the model's server answered 401: Incorrect API key provided: bad\n\
the model's server answered no JSON: this is not json\n\
false\n\
llm\n\
false\n\
llm\n"
    );
    for mode in [&["run", "-i"][..], &["run"][..]] {
        let out = Command::new(env!("CARGO_BIN_EXE_crow"))
            .args(mode)
            .arg("main.co")
            .arg(port.to_string())
            .current_dir(&dir)
            .env("CROW_BUILD_PROFILE", "dev")
            .env("ANTHROPIC_BASE_URL", format!("http://127.0.0.1:{}", port))
            .env("ANTHROPIC_API_KEY", "test")
            .env("OPENAI_API_KEY", "test")
            .env("CROW_LLM_TEST_BAD_KEY", "bad")
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), said, "{:?}: {}", mode, String::from_utf8_lossy(&out.stderr));
    }
}
