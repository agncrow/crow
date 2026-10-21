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

use serde_json::{json, Value};
use std::io::{Read, Write};
use std::process::Command;

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

/// The events in chunks of uneven widths, each chunk written on its own
/// after a pause: an event straddles the reads too, not only the chunks (a
/// reader that let go of the half event it held, to read the rest, had
/// lost the event; one write of the whole body had never shown it).
fn respond_stream(mut stream: std::net::TcpStream, events: &str, width: usize) {
    let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n");
    let _ = stream.flush();
    for piece in straddled(events, width) {
        std::thread::sleep(std::time::Duration::from_millis(5));
        let _ = write!(stream, "{:x}\r\n{}\r\n", piece.len(), piece);
        let _ = stream.flush();
    }
    let _ = stream.write_all(b"0\r\n\r\n");
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

/// A reply of text in the format the path names, with the stop reason the
/// loop reads as `end` or `length`.
fn reply_text(anthropic: bool, text: &str, stop: &str) -> String {
    if anthropic {
        let reason = if stop == "length" { "max_tokens" } else { "end_turn" };
        json!({"content": [{"type": "text", "text": text}], "stop_reason": reason, "usage": {"input_tokens": 21, "output_tokens": 3}}).to_string()
    } else {
        let reason = if stop == "length" { "length" } else { "stop" };
        json!({"choices": [{"message": {"role": "assistant", "content": text}, "finish_reason": reason}], "usage": {"prompt_tokens": 21, "completion_tokens": 3}}).to_string()
    }
}

/// A reply of calls — one block or tool_call each, ids numbered — in the
/// format the path names.
fn reply_calls(anthropic: bool, calls: &[(&str, Value)]) -> String {
    if anthropic {
        let blocks: Vec<Value> = calls.iter().enumerate().map(|(i, (n, a))| json!({"type": "tool_use", "id": format!("toolu_{}", i + 1), "name": n, "input": a})).collect();
        json!({"content": blocks, "stop_reason": "tool_use", "usage": {"input_tokens": 12, "output_tokens": 15}}).to_string()
    } else {
        let tcs: Vec<Value> = calls.iter().enumerate().map(|(i, (n, a))| json!({"id": format!("call_{}", i + 1), "type": "function", "function": {"name": n, "arguments": a.to_string()}})).collect();
        json!({"choices": [{"message": {"role": "assistant", "content": null, "tool_calls": tcs}, "finish_reason": "tool_calls"}], "usage": {"prompt_tokens": 7, "completion_tokens": 5}}).to_string()
    }
}

/// The tool results the last turn carries — each result's text and
/// whether it was flagged an error (Anthropic's `is_error`; the
/// OpenAI-compatible wire has no flag).
fn told_results(messages: &[Value], anthropic: bool) -> Vec<(String, bool)> {
    if anthropic {
        let Some(last) = messages.last().filter(|m| m["role"] == "user") else { return Vec::new() };
        last["content"]
            .as_array()
            .map(|blocks| blocks.iter().filter(|b| b["type"] == "tool_result").map(|b| (b["content"].as_str().unwrap_or("").to_string(), b["is_error"].as_bool().unwrap_or(false))).collect())
            .unwrap_or_default()
    } else {
        messages.iter().rev().take_while(|m| m["role"] == "tool").map(|m| (m["content"].as_str().unwrap_or("").to_string(), false)).collect::<Vec<_>>().into_iter().rev().collect()
    }
}

/// Every tool result the conversation holds, in order, each with whether
/// it was flagged an error.
fn all_told(messages: &[Value], anthropic: bool) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    for m in messages {
        if anthropic {
            if m["role"] != "user" {
                continue;
            }
            if let Some(blocks) = m["content"].as_array() {
                for b in blocks.iter().filter(|b| b["type"] == "tool_result") {
                    out.push((b["content"].as_str().unwrap_or("").to_string(), b["is_error"].as_bool().unwrap_or(false)));
                }
            }
        } else if m["role"] == "tool" {
            out.push((m["content"].as_str().unwrap_or("").to_string(), false));
        }
    }
    out
}

/// The system prompt sent, in either format.
fn system_of(v: &Value, messages: &[Value], anthropic: bool) -> String {
    if anthropic {
        v["system"].as_str().unwrap_or("").to_string()
    } else {
        messages.iter().find(|m| m["role"] == "system").and_then(|m| m["content"].as_str()).unwrap_or("").to_string()
    }
}

/// A scripted run for the agent loop: a question beginning `act:` names
/// the scenario, the model's turns so far say which step this is, and the
/// final text says back what the model was told (`heard: …`, each failed
/// result marked `(error)` where the wire carries the flag), so the test
/// sees what the model heard. None for any other question.
fn scripted(path: &str, body: &str) -> Option<String> {
    let anthropic = path == "/v1/messages";
    if !anthropic && path != "/v1/chat/completions" {
        return None;
    }
    let v: Value = serde_json::from_str(body).ok()?;
    let messages = v["messages"].as_array().cloned().unwrap_or_default();
    let user_text = |m: &Value| -> String {
        if anthropic {
            m["content"].as_array().and_then(|b| b.iter().find(|x| x["type"] == "text")).and_then(|x| x["text"].as_str()).unwrap_or("").to_string()
        } else {
            m["content"].as_str().unwrap_or("").to_string()
        }
    };
    let first = messages.iter().find(|m| m["role"] == "user")?;
    let question = user_text(first);
    let last_user = messages.iter().rev().find(|m| m["role"] == "user").map(user_text).unwrap_or_default();
    // (a compacted conversation starts from a summary; a summary is asked
    // with no tools and the question gone)
    if last_user.starts_with("Summarize the conversation") {
        return Some(reply_text(anthropic, "the task: the weather (a summary)", "end"));
    }
    if question.starts_with("The conversation so far, summarized:") {
        return Some(reply_text(anthropic, &format!("heard: resumed from {}", question.lines().nth(1).unwrap_or("")), "end"));
    }
    let scenario = question.strip_prefix("act: ")?.to_string();
    let everything = {
        let parts: Vec<String> = all_told(&messages, anthropic).iter().map(|(t, e)| if *e { format!("{} (error)", t) } else { t.clone() }).collect();
        format!("seen: {}", parts.join(" | "))
    };
    let listed = if system_of(&v, &messages, anthropic).contains("crow-parity-test: ") { " | listed" } else { "" };
    let round = messages.iter().filter(|m| m["role"] == "assistant").count();
    let heard = {
        let parts: Vec<String> = told_results(&messages, anthropic).iter().map(|(t, e)| if *e { format!("{} (error)", t) } else { t.clone() }).collect();
        format!("heard: {}", parts.join(" | "))
    };
    let lisbon = json!({"city": "Lisbon"});
    Some(match scenario.as_str() {
        "weather" if round == 0 => reply_calls(anthropic, &[("weather", lisbon)]),
        "forever" => reply_calls(anthropic, &[("weather", lisbon)]),
        "nothing" if round == 0 => reply_calls(anthropic, &[("nothing", json!({}))]),
        "bad" if round == 0 => reply_calls(anthropic, &[("forecast", json!({"city": "Lisbon", "days": "many"}))]),
        "fail" if round == 0 => reply_calls(anthropic, &[("weather", json!({"city": "Nowhere"}))]),
        "two" if round == 0 => reply_calls(anthropic, &[("weather", lisbon), ("weather", json!({"city": "Porto"}))]),
        "length" => reply_text(anthropic, "cut off", "length"),
        "keep" if round < 3 => reply_calls(anthropic, &[("weather", lisbon)]),
        "keep" => reply_text(anthropic, &everything, "end"),
        "compact" if round == 0 => reply_calls(anthropic, &[("weather", lisbon)]),
        "skills" if round == 0 => reply_calls(anthropic, &[("skill", json!({"name": "crow-parity-test"}))]),
        "skills" if round == 1 => reply_calls(anthropic, &[("skill_file", json!({"name": "crow-parity-test", "path": "references/parity.md"}))]),
        "skills" if round == 2 => reply_calls(anthropic, &[("skill_file", json!({"name": "crow-parity-test", "path": "../../secrets.txt"})), ("skill", json!({"name": "nothing"}))]),
        "skills" => reply_text(anthropic, &format!("{}{}", everything, listed), "end"),
        "mcp" if round == 0 => reply_calls(anthropic, &[("echo", json!({"words": "over mcp"})), ("fail", json!({}))]),
        "mcp" => reply_text(anthropic, &everything, "end"),
        // (a parent delegates: the tool `research` runs a loop of its own
        // against this same server, asking the `weather` scenario; the
        // `locked` researcher's model refuses its key, so its run fails and
        // the parent hears why)
        "delegate" if round == 0 => reply_calls(anthropic, &[("research", json!({"question": "act: weather"}))]),
        "locked" if round == 0 => reply_calls(anthropic, &[("research_locked", json!({"question": "anything"}))]),
        _ => reply_text(anthropic, &heard, "end"),
    })
}

/// A stand-in MCP server at `/mcp`: an `echo` tool and one that fails,
/// listed and called by the loop's `servers = [s]`.
fn mcp_answer(body: &str) -> String {
    let sent: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let id = sent["id"].clone();
    match sent["method"].as_str().unwrap_or("") {
        "tools/list" => json!({"jsonrpc": "2.0", "id": id, "result": {"resultType": "complete", "tools": [
            {"name": "echo", "description": "says its words back", "inputSchema": {"type": "object", "properties": {"words": {"type": "string"}}, "required": ["words"]}},
            {"name": "fail", "description": "always fails", "inputSchema": {"type": "object", "additionalProperties": false}}
        ]}}).to_string(),
        "tools/call" => {
            let name = sent["params"]["name"].as_str().unwrap_or("");
            let result = if name == "echo" {
                json!({"resultType": "complete", "content": [{"type": "text", "text": format!("echo: {}", sent["params"]["arguments"]["words"].as_str().unwrap_or(""))}], "isError": false})
            } else {
                json!({"resultType": "complete", "content": [{"type": "text", "text": "it failed, as it always does"}], "isError": true})
            };
            json!({"jsonrpc": "2.0", "id": id, "result": result}).to_string()
        }
        other => json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": format!("Method not found: {}", other)}}).to_string(),
    }
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
    if let Some(reply) = scripted(&path, &body) {
        return respond(stream, "200 OK", &reply);
    }
    if path == "/mcp" {
        return respond(stream, "200 OK", &mcp_answer(&body));
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
r = chat[m, talk, system = 'one word', tokens = 32]
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

/// The agent loop against the scripted model (spec, llm: The loop is the
/// module's): a tool called once and answered, a model that keeps calling
/// stopped by `rounds` and by `budget`, an unknown name, a wrong argument,
/// a tool's own failure and two calls in one reply each told as the loop
/// tells them (the wrong argument against a second tool list: the loop is
/// copied per list), a reply cut by its length, and the same in
/// Anthropic's format, where a failed result reaches the wire as
/// `is_error`.
const AGENT: &str = "use llm, mcp

port = args[][0]
m = reach['openai://127.0.0.1:`port`/test-model']

--- the current weather in a city
fn weather[city]
    if city eq 'Nowhere'
        return error['no station in `city`', 'weather']
    return 'sunny in `upper[city]`'

--- the forecast for a city, for a number of days
fn forecast[city, days = 2]
    return '`days` days of sun in `upper[city]`'

--- says its words back, from the program (a name an MCP server also offers)
fn echo[words]
    return 'echo `trim[words]`'

--- the program's gate: the weather may be asked for Lisbon alone
fn only_lisbon[call]
    if str[call.name] ne 'weather'
        return true
    return str[call.args.city] eq 'Lisbon'

--- what a smaller agent finds out about a question, with the weather as
--- its only tool: its answer, or why its run failed (a subagent is a
--- tool that runs the loop; a function sees no top-level name, so it
--- reaches the model itself)
fn research[question]
    port = args[][0]
    inner = act[reach['openai://127.0.0.1:`port`/test-model'], ask[trim[question]], tools = [weather], rounds = 4]
    return inner.text

--- a researcher whose model refuses its key: its run fails, and it answers
--- a failure in words of its own, which the parent hears as one
fn research_locked[question]
    port = args[][0]
    inner = act[reach['openai://127.0.0.1:`port`/test-model?key=CROW_LLM_TEST_BAD_KEY'], ask[trim[question]], tools = [weather], rounds = 4]
    if not inner.ok
        return error['the researcher could not run: `inner.reason`', 'research']
    return inner.text

done = act[m, ask['act: weather'], tools = [weather]]
log[done.text]
log[done.stop]
log[done.rounds]
log[done.usage.input]
log[done.usage.output]
log[len[done.talk]]
log[done.steps.what]
log[done.steps.name]
log[done.steps.how]
log[done.steps.input]
log[len[done.steps.ms]]

more = act[m, ask['act: forever'], tools = [weather], rounds = 3]
log[more.stop]
log[more.rounds]
log[len[more.talk]]
log[more.steps.how]

none = act[m, ask['act: nothing'], tools = [weather]]
log[none.text]
log[none.steps.how]

bad = act[m, ask['act: bad'], tools = [weather, forecast]]
log[bad.text]
log[bad.steps.how]

broken = act[m, ask['act: fail'], tools = [weather]]
log[broken.text]
log[broken.steps.how]

two = act[m, ask['act: two'], tools = [weather]]
log[two.text]
log[len[two.steps.what]]

gated = act[m, ask['act: two'], tools = [weather], allow = [only_lisbon]]
log[gated.text]
log[gated.steps.how]
ungated = act[m, ask['act: weather'], tools = [weather], allow = [weather]]
log[ungated.reason]

delegated = act[m, ask['act: delegate'], tools = [research, weather]]
log[delegated.text]
log[delegated.steps.name]
log[delegated.steps.how]
log[len[delegated.talk]]
locked = act[m, ask['act: locked'], tools = [research_locked, weather]]
log[locked.text]
log[locked.steps.how]
log[locked.talk[2].failed]

tight = act[m, ask['act: forever'], tools = [weather], budget = 10]
log[tight.stop]
log[tight.rounds]

cut = act[m, ask['act: length'], tools = [weather]]
log[cut.stop]
log[cut.text]

kept = act[m, ask['act: keep'], tools = [weather], keep = 1]
log[kept.text]
log[kept.rounds]
log[kept.talk[2].text]
log[kept.talk[2].cut]

packed = act[m, ask['act: compact'], tools = [weather], compact = 1]
log[packed.text]
log[packed.steps.what]
log[len[packed.talk]]
log[packed.rounds]

skilled = act[m, ask['act: skills'], tools = [weather], skills = 'skills', system = 'be brief']
log[skilled.text]
log[skilled.steps.how]
log[len[skilled.steps.what]]
unskilled = act[m, ask['act: skills'], tools = [weather], skills = 'bad_skills']
log[unskilled.ok]
log[unskilled.reason]

mcp_s = server['http://127.0.0.1:`port`/mcp']
over = act[m, ask['act: mcp'], tools = [weather], servers = [mcp_s]]
log[over.text]
log[over.steps.name]
log[over.steps.how]
taken = act[m, ask['act: mcp'], tools = [weather, echo], servers = [mcp_s]]
log[taken.reason]
std_s = server[command = args[][1]]
over_stdio = act[m, ask['act: mcp'], tools = [weather], servers = [std_s]]
log[over_stdio.text]
log[over_stdio.steps.name]
log[over_stdio.steps.how]
both = act[m, ask['act: mcp'], tools = [weather, echo], servers = [std_s]]
log[both.reason]

a = reach['anthropic://claude-test']
da = act[a, ask['act: weather'], tools = [weather]]
log[da.text]
log[da.stop]
log[da.usage.input]
log[da.steps.how]
na = act[a, ask['act: nothing'], tools = [weather]]
log[na.text]
fa = act[a, ask['act: fail'], tools = [weather]]
log[fa.text]
ga = act[a, ask['act: two'], tools = [weather], allow = [only_lisbon]]
log[ga.text]
dla = act[a, ask['act: delegate'], tools = [research, weather]]
log[dla.text]
log[dla.steps.how]
la = act[a, ask['act: locked'], tools = [research_locked, weather]]
log[la.text]
";

#[test]
fn an_agent_runs_its_tools_in_a_loop_in_both_modes() {
    let port = model_server();
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/crow_llm_server_agent");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.co"), AGENT).unwrap();
    let skill = dir.join("skills/crow-parity-test");
    std::fs::create_dir_all(skill.join("references")).unwrap();
    std::fs::write(skill.join("SKILL.md"), "---\nname: crow-parity-test\ndescription: Writes a parity program. Use when a feature needs a test under tests/parity/.\n---\n# A parity program\n\n1. Take the next number.\n").unwrap();
    std::fs::write(skill.join("references/parity.md"), "Both modes must print the same lines.\n").unwrap();
    let bad = dir.join("bad_skills/Bad-Name");
    std::fs::create_dir_all(&bad).unwrap();
    std::fs::write(bad.join("SKILL.md"), "---\nname: Bad-Name\ndescription: a skill whose name is no name\n---\nbody\n").unwrap();
    // an MCP server over stdio: a Crow program the loop starts as a
    // command (in a directory of its own, since a `.crow_build` is one
    // program's), offering the stand-in's two tools
    let stdio = dir.join("stdio");
    std::fs::create_dir_all(&stdio).unwrap();
    std::fs::write(stdio.join("server.co"), STDIO_SERVER).unwrap();
    let command = format!("cd '{}' && '{}' run -i server.co", stdio.display(), env!("CARGO_BIN_EXE_crow"));
    let said = "heard: sunny in LISBON\n\
end\n\
2\n\
28\n\
8\n\
4\n\
['chat', 'tool', 'chat']\n\
['test-model', 'weather', 'test-model']\n\
['tool', 'ok', 'end']\n\
[7, 0, 21]\n\
3\n\
rounds\n\
3\n\
7\n\
['tool', 'ok', 'tool', 'ok', 'tool', 'ok']\n\
heard: no tool is named nothing\n\
['tool', 'unknown', 'end']\n\
heard: a parsed value holds text 'many' where the program reads an int (forecast[]'s 'days')\n\
['tool', 'arguments', 'end']\n\
heard: no station in Nowhere\n\
['tool', 'failed', 'end']\n\
heard: sunny in LISBON | sunny in PORTO\n\
4\n\
heard: sunny in LISBON | weather is not allowed by the program\n\
['tool', 'ok', 'refused', 'end']\n\
a gate takes the call and answers a bool: weather takes 1 and answers text\n\
heard: heard: sunny in LISBON\n\
['test-model', 'research', 'test-model']\n\
['tool', 'ok', 'end']\n\
4\n\
heard: the researcher could not run: the model's server answered 401: Incorrect API key provided: bad\n\
['tool', 'failed', 'end']\n\
true\n\
budget\n\
1\n\
length\n\
cut off\n\
seen: [a result of 15 characters, cut] | [a result of 15 characters, cut] | sunny in LISBON\n\
4\n\
[a result of 15 characters, cut]\n\
true\n\
heard: resumed from the task: the weather (a summary)\n\
['chat', 'tool', 'compact', 'chat']\n\
2\n\
2\n\
seen: # A parity program\n\n1. Take the next number. | Both modes must print the same lines.\n | a skill's file is named by its path inside the skill: ../../secrets.txt | no skill is named nothing | listed\n\
['tool', 'ok', 'tool', 'ok', 'tool', 'failed', 'unknown', 'end']\n\
8\n\
false\n\
the skill Bad-Name's name 'Bad-Name' holds 'B', and a name holds lowercase letters, digits and hyphens\n\
seen: echo: over mcp | it failed, as it always does\n\
['test-model', 'echo', 'fail', 'test-model']\n\
['tool', 'ok', 'failed', 'end']\n\
the MCP server http://127.0.0.1:@PORT@/mcp offers a tool named echo, which another tool has: the names must differ\n\
seen: echo: over mcp | it failed, as it always does\n\
['test-model', 'echo', 'fail', 'test-model']\n\
['tool', 'ok', 'failed', 'end']\n\
the MCP server @STDIO@ offers a tool named echo, which another tool has: the names must differ\n\
heard: sunny in LISBON\n\
end\n\
33\n\
['tool', 'ok', 'end']\n\
heard: no tool is named nothing (error)\n\
heard: no station in Nowhere (error)\n\
heard: sunny in LISBON | weather is not allowed by the program (error)\n\
heard: heard: sunny in LISBON\n\
['tool', 'ok', 'end']\n\
heard: the researcher could not run: the model's server answered 401: Incorrect API key provided: bad (error)\n";
    for mode in [&["run", "-i"][..], &["run"][..]] {
        let out = Command::new(env!("CARGO_BIN_EXE_crow"))
            .args(mode)
            .arg("main.co")
            .arg(port.to_string())
            .arg(&command)
            .current_dir(&dir)
            .env("CROW_BUILD_PROFILE", "dev")
            .env("ANTHROPIC_BASE_URL", format!("http://127.0.0.1:{}", port))
            .env("ANTHROPIC_API_KEY", "test")
            .env("OPENAI_API_KEY", "test")
            .env("CROW_LLM_TEST_BAD_KEY", "bad")
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).to_string(), said.replace("@PORT@", &port.to_string()).replace("@STDIO@", &command), "{:?}: {}", mode, String::from_utf8_lossy(&out.stderr));
    }
}

/// The stand-in's two tools as a Crow program serving over its standard
/// streams (`listen`), started by the loop's client as a command.
const STDIO_SERVER: &str = "use mcp

--- says its words back
fn echo[words]
    return 'echo: `trim[words]`'

--- always fails
fn fail[]
    return error['it failed, as it always does', 'tool']

listen[tools = [echo, fail], name = 'stand-in']
";
