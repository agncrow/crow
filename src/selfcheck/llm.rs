//! The llm family (`use llm`): conversations built and read with no model —
//! `ask`, `said`, `told`, `reach` and their refusals — and a model of the
//! check's own (CROW_SELF_LLM) that speaks both wire formats, the
//! OpenAI-compatible chat completions and embeddings and Anthropic's
//! messages, each plain and streamed as server-sent events over chunked
//! transfer encoding (an event straddling the chunks). Its every answer
//! says back what it was sent — the user's last words, the system prompt,
//! the tokens at most and the temperature, a tool's result — so the
//! reference knows what each reply holds, and a request the module wrote
//! wrong answers otherwise.

use super::Case;
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::time::Duration;

/// One connection to the model: a request read whole (by its length, or
/// its chunks), answered by its path, the connection closed.
pub(super) fn serve_model(mut s: std::net::TcpStream) {
    let _ = s.set_read_timeout(Some(Duration::from_secs(60)));
    let Some((path, body)) = read_request(&mut s) else { return };
    let sent: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    if path.starts_with("/refuse") {
        return respond(&mut s, "401 Unauthorized", r#"{"error": {"message": "no key here"}}"#);
    }
    if path.starts_with("/broken") {
        return respond(&mut s, "200 OK", "this is not json");
    }
    if path.ends_with("/embeddings") {
        // (a row per text: its length in characters, then two numbers)
        let rows: Vec<Value> = sent["input"].as_array().map(|xs| xs.iter().map(|x| json!({"embedding": [x.as_str().map_or(0, |t| t.chars().count()) as f64, 0.5, -0.25]})).collect()).unwrap_or_default();
        return respond(&mut s, "200 OK", &json!({"data": rows, "usage": {"prompt_tokens": 1}}).to_string());
    }
    let streamed = sent["stream"].as_bool().unwrap_or(false);
    if path.ends_with("/chat/completions") {
        let r = openai_heard(&sent);
        if streamed {
            return respond_events(&mut s, &openai_events(&r));
        }
        return respond(&mut s, "200 OK", &openai_reply(&r).to_string());
    }
    if path == "/v1/messages" {
        let r = anthropic_heard(&sent);
        if streamed {
            return respond_events(&mut s, &anthropic_events(&r));
        }
        return respond(&mut s, "200 OK", &anthropic_reply(&r).to_string());
    }
    respond(&mut s, "404 Not Found", r#"{"error": {"message": "no such path"}}"#)
}

/// A request's path and body: the head to its blank line, the body by its
/// length or, sent in chunks, to its last chunk.
fn read_request(s: &mut std::net::TcpStream) -> Option<(String, Vec<u8>)> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 65536];
    let mut more = |buf: &mut Vec<u8>| -> Option<()> {
        let n = s.read(&mut chunk).ok().filter(|&n| n > 0)?;
        buf.extend_from_slice(&chunk[..n]);
        Some(())
    };
    let head_end = loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i;
        }
        more(&mut buf)?;
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let header = |name: &str| head.lines().find_map(|l| l.split_once(':').filter(|(k, _)| k.trim().eq_ignore_ascii_case(name)).map(|(_, v)| v.trim().to_string()));
    let path = head.lines().next()?.split_whitespace().nth(1)?.to_string();
    let start = head_end + 4;
    if header("transfer-encoding").is_some_and(|v| v.eq_ignore_ascii_case("chunked")) {
        loop {
            if let Some(body) = unchunked(&buf[start..]) {
                return Some((path, body));
            }
            more(&mut buf)?;
        }
    }
    let length: usize = header("content-length").and_then(|v| v.parse().ok()).unwrap_or(0);
    while buf.len() < start + length {
        more(&mut buf)?;
    }
    Some((path, buf[start..start + length].to_vec()))
}

/// A chunked body decoded, or None while its last chunk has not come.
fn unchunked(raw: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut at = 0;
    loop {
        let end = raw.get(at..)?.windows(2).position(|w| w == b"\r\n")? + at;
        let size = usize::from_str_radix(std::str::from_utf8(&raw[at..end]).ok()?.split(';').next()?.trim(), 16).ok()?;
        at = end + 2;
        if size == 0 {
            return Some(out);
        }
        out.extend_from_slice(raw.get(at..at + size)?);
        at += size + 2;
    }
}

fn respond(s: &mut std::net::TcpStream, status: &str, body: &str) {
    let _ = write!(s, "HTTP/1.1 {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", status, body.len());
    let _ = s.write_all(body.as_bytes());
}

/// Server-sent events in chunks of 23 and 26 bytes by turns, each cut on a
/// character's boundary: an event straddles the chunks, as a server's
/// writes fall where they fall.
fn respond_events(s: &mut std::net::TcpStream, events: &str) {
    let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n");
    let mut rest = events;
    let mut width = 23;
    while !rest.is_empty() {
        let mut cut = width.min(rest.len());
        while !rest.is_char_boundary(cut) {
            cut -= 1;
        }
        let _ = write!(s, "{:x}\r\n", cut);
        let _ = s.write_all(&rest.as_bytes()[..cut]);
        let _ = s.write_all(b"\r\n");
        rest = &rest[cut..];
        width = if width == 23 { 26 } else { 23 };
    }
    let _ = s.write_all(b"0\r\n\r\n");
}

/// What the model makes of a request: its answer's text (the words heard
/// said back, or a tool's result read back), or a call of the first tool
/// offered — its arguments the user's last words — and the tokens in and
/// out (the turns it was sent, the words it answers).
struct Heard {
    text: String,
    call: Option<(String, String)>,
    input: usize,
    output: usize,
}

fn heard(user: &str, system: &str, most: &Value, temperature: &Value, told: Option<&str>, tool: Option<&str>, turns: usize) -> Heard {
    if let Some(t) = told {
        let text = format!("told {}", t);
        return Heard { output: text.split_whitespace().count(), text, call: None, input: turns };
    }
    if let Some(name) = tool {
        return Heard { text: String::new(), call: Some((name.to_string(), json!({"x": user}).to_string())), input: turns, output: 1 };
    }
    let text = format!("heard {} | {} | {} | {}", user, if system.is_empty() { "-" } else { system }, most, temperature);
    Heard { output: text.split_whitespace().count(), text, call: None, input: turns }
}

/// An OpenAI-compatible request read: the system prompt is its first
/// message, a tool's result a message of its own (the last, when the
/// model is to read it back).
fn openai_heard(sent: &Value) -> Heard {
    let messages = sent["messages"].as_array().cloned().unwrap_or_default();
    let system = messages.iter().find(|m| m["role"] == "system").and_then(|m| m["content"].as_str()).unwrap_or("");
    let user = messages.iter().rev().find(|m| m["role"] == "user").and_then(|m| m["content"].as_str()).unwrap_or("");
    let told = messages.last().filter(|m| m["role"] == "tool").and_then(|m| m["content"].as_str());
    let tool = sent["tools"].as_array().and_then(|t| t.first()).and_then(|t| t["function"]["name"].as_str());
    heard(user, system, &sent["max_tokens"], &sent["temperature"], told, tool, messages.len())
}

fn openai_reply(r: &Heard) -> Value {
    let usage = json!({"prompt_tokens": r.input, "completion_tokens": r.output});
    match &r.call {
        Some((name, args)) => json!({"choices": [{"message": {"role": "assistant", "content": null, "tool_calls": [{"id": "call_self", "type": "function", "function": {"name": name, "arguments": args}}]}, "finish_reason": "tool_calls"}], "usage": usage}),
        None => json!({"choices": [{"message": {"role": "assistant", "content": r.text}, "finish_reason": "stop"}], "usage": usage}),
    }
}

/// The reply as OpenAI streams it: the role, the words one by one (a
/// keep-alive comment among them), the reason it stopped, the usage, done;
/// a call's arguments in two pieces.
fn openai_events(r: &Heard) -> String {
    let mut lines: Vec<Value> = Vec::new();
    let stop = match &r.call {
        Some((name, args)) => {
            let half = args.char_indices().nth(args.chars().count() / 2).map_or(args.len(), |(i, _)| i);
            lines.push(json!({"choices": [{"delta": {"role": "assistant", "content": null, "tool_calls": [{"index": 0, "id": "call_self", "type": "function", "function": {"name": name, "arguments": ""}}]}, "finish_reason": null}]}));
            for piece in [&args[..half], &args[half..]] {
                lines.push(json!({"choices": [{"delta": {"tool_calls": [{"index": 0, "function": {"arguments": piece}}]}, "finish_reason": null}]}));
            }
            "tool_calls"
        }
        None => {
            lines.push(json!({"choices": [{"delta": {"role": "assistant", "content": ""}, "finish_reason": null}]}));
            for piece in r.text.split_inclusive(' ') {
                lines.push(json!({"choices": [{"delta": {"content": piece}, "finish_reason": null}]}));
            }
            "stop"
        }
    };
    lines.push(json!({"choices": [{"delta": {}, "finish_reason": stop}]}));
    lines.push(json!({"choices": [], "usage": {"prompt_tokens": r.input, "completion_tokens": r.output}}));
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

/// A user turn's words, as Anthropic's messages carry them: the content
/// itself, or its text blocks.
fn anthropic_words(content: &Value) -> Option<String> {
    if let Some(t) = content.as_str() {
        return Some(t.to_string());
    }
    let blocks = content.as_array()?;
    let texts: Vec<&str> = blocks.iter().filter(|b| b["type"] == "text").filter_map(|b| b["text"].as_str()).collect();
    (!texts.is_empty()).then(|| texts.concat())
}

/// An Anthropic request read: the system prompt a field of its own, a
/// tool's result a block of the last user turn.
fn anthropic_heard(sent: &Value) -> Heard {
    let messages = sent["messages"].as_array().cloned().unwrap_or_default();
    let user = messages.iter().rev().filter(|m| m["role"] == "user").find_map(|m| anthropic_words(&m["content"])).unwrap_or_default();
    let told = messages.last().filter(|m| m["role"] == "user").and_then(|m| m["content"].as_array()).and_then(|blocks| blocks.iter().rev().find(|b| b["type"] == "tool_result")).and_then(|b| anthropic_words(&b["content"]));
    let tool = sent["tools"].as_array().and_then(|t| t.first()).and_then(|t| t["name"].as_str());
    heard(&user, sent["system"].as_str().unwrap_or(""), &sent["max_tokens"], &sent["temperature"], told.as_deref(), tool, messages.len())
}

fn anthropic_reply(r: &Heard) -> Value {
    let usage = json!({"input_tokens": r.input, "output_tokens": r.output});
    match &r.call {
        Some((name, args)) => {
            let input: Value = serde_json::from_str(args).unwrap_or(Value::Null);
            json!({"content": [{"type": "tool_use", "id": "toolu_self", "name": name, "input": input}], "stop_reason": "tool_use", "usage": usage})
        }
        None => json!({"content": [{"type": "text", "text": r.text}], "stop_reason": "end_turn", "usage": usage}),
    }
}

/// The reply as Anthropic streams it: the message's start with its input
/// tokens, a block's start, its pieces (a ping among them), its stop, the
/// reason and the output tokens, the message's stop.
fn anthropic_events(r: &Heard) -> String {
    let mut events: Vec<(&str, Value)> = vec![("message_start", json!({"type": "message_start", "message": {"usage": {"input_tokens": r.input, "output_tokens": 1}}}))];
    let stop = match &r.call {
        Some((name, args)) => {
            let half = args.char_indices().nth(args.chars().count() / 2).map_or(args.len(), |(i, _)| i);
            events.push(("content_block_start", json!({"type": "content_block_start", "index": 0, "content_block": {"type": "tool_use", "id": "toolu_self", "name": name, "input": {}}})));
            for piece in [&args[..half], &args[half..]] {
                events.push(("content_block_delta", json!({"type": "content_block_delta", "index": 0, "delta": {"type": "input_json_delta", "partial_json": piece}})));
            }
            "tool_use"
        }
        None => {
            events.push(("content_block_start", json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}})));
            events.push(("ping", json!({"type": "ping"})));
            for piece in r.text.split_inclusive(' ') {
                events.push(("content_block_delta", json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": piece}})));
            }
            "end_turn"
        }
    };
    events.push(("content_block_stop", json!({"type": "content_block_stop", "index": 0})));
    events.push(("message_delta", json!({"type": "message_delta", "delta": {"stop_reason": stop}, "usage": {"output_tokens": r.output}})));
    events.push(("message_stop", json!({"type": "message_stop"})));
    events.iter().map(|(e, d)| format!("event: {}\ndata: {}\n\n", e, d)).collect()
}

/// The llm family's cases: a conversation begun, added to by the user, by
/// the model's reply and by a tool's result, and each refused where its
/// value is not what it says; a model reached by each URL's form and
/// refused by each mistake; then, against the check's model, a reply, its
/// options sent, a tool called and its result read back, a reply followed
/// piece by piece (a call's too), embeddings, the agent loop (the first
/// tool called, its result said back, the steps recorded), each again in
/// Anthropic's format, and a server that refuses, one that answers no
/// JSON and one nobody answers.
pub(super) fn llm_cases() -> Vec<Case> {
    let reply = r#"json['{"text": "hello", "calls": [], "usage": {"input": 1, "output": 1}, "stop": "end"}']"#;
    let called = r#"json['{"text": "", "calls": [{"id": "c1", "name": "f", "args": {"x": 1}}], "usage": {"input": 1, "output": 1}, "stop": "tool"}']"#;
    let pure: Vec<(&str, String)> = vec![
        ("ask", "log[ask['hi there']]".into()),
        ("asked", "t = ask['hi']\nlog[asked[t, 'and you']]".into()),
        ("said", format!("t = ask['hi']\nlog[said[t, {}]]", reply)),
        ("told", format!("t = ask['hi']\nr = {}\nt2 = said[t, r]\nlog[told[t2, r.calls[0], 'done']]\nlog[len[t2]]", called)),
        ("said-no-reply", "r = said[ask['hi'], json['{\"a\": 1}']]\nlog[r.ok]\nlog[r.reason]".into()),
        ("told-no-call", "r = told[ask['hi'], json['{\"a\": 1}'], 'x']\nlog[r.ok]\nlog[r.reason]".into()),
        ("no-talk", "r = asked[json['{\"a\": 1}'], 'x']\nlog[r.ok]\nlog[r.reason]".into()),
        ("reach", "m = reach['openai://localhost:11434/llama3.2']\nlog[m.provider]\nlog[m.model]\nlog[m.base]\nlog[m.path]\nlog[m.key]".into()),
        ("reach-options", "m = reach['openai://127.0.0.1:8000/org/model?path=/api/v1&key=SELF_KEY']\nlog[m.model]\nlog[m.base]\nlog[m.path]\nlog[m.key]".into()),
        ("reach-no-key", "log[reach['openai://api.example.com/gpt'].reason]".into()),
        ("reach-anthropic-no-key", "log[reach['anthropic://claude-x'].reason]".into()),
        ("reach-no-model", "log[reach['anthropic://'].reason]".into()),
        ("reach-host-no-model", "log[reach['openai://localhost:11434'].reason]".into()),
        ("reach-scheme", "log[reach['http://x'].reason]".into()),
    ];
    let open = "self_p = env['CROW_SELF_LLM', '9']\nm = reach['openai://127.0.0.1:`self_p`/self-model']";
    let anthropic = "a = reach['anthropic://self-model?key=CROW_SELF_LLM']";
    let follow = |s: &str| format!("piece = next[{0}]\nwhile piece.ok and len[piece] gt 0\n    log['piece: [`piece`]']\n    piece = next[{0}]\nwhole = reply[{0}]", s);
    let tool_flow = |m: &str| {
        format!(
            "t = ask['hi']\nr = chat[{0}, t, tools = [self_tool_@N]]\nlog[r.stop]\nlog[starts[r.calls[0].name, 'self_tool_']]\nlog[r.calls[0].args.x]\nt2 = told[said[t, r], r.calls[0], self_tool_@N[str[r.calls[0].args.x]]]\nlog[len[t2]]\nr2 = chat[{0}, t2, tools = [self_tool_@N]]\nlog[r2.text]\nlog[r2.stop]\nlog[r2.usage.input]",
            m
        )
    };
    let live: Vec<(&str, String)> = vec![
        ("chat", format!("{}\nr = chat[m, ask['hi there']]\nlog[r.text]\nlog[r.stop]\nlog[r.usage.input]\nlog[r.usage.output]\nlog[len[r.calls]]", open)),
        ("chat-options", format!("{}\nr = chat[m, ask['hi'], system = 'be brief', tokens = 16, temperature = 0.5]\nlog[r.text]\nlog[r.usage.input]", open)),
        ("tool", format!("{}\n{}", open, tool_flow("m"))),
        ("act", format!("{}\nd = act[m, ask['hi'], tools = [self_tool_@N]]\nlog[d.text]\nlog[d.stop]\nlog[d.rounds]\nlog[len[d.talk]]\nlog[d.steps.what]\nlog[d.steps.how]\nlog[d.usage.input]\nlog[d.usage.output]", open)),
        ("follow", format!("{}\ns = follow[m, ask['one two']]\n{}\nlog[whole.text]\nlog[whole.stop]\nlog[whole.usage.input]\nlog[whole.usage.output]", open, follow("s"))),
        ("follow-tool", format!("{}\ns = follow[m, ask['hi'], tools = [self_tool_@N]]\n{}\nlog[whole.stop]\nlog[starts[whole.calls[0].name, 'self_tool_']]\nlog[whole.calls[0].args.x]", open, follow("s"))),
        ("embed", format!("{}\nlog[embed[m, ['one', 'three']]]", open)),
        ("anthropic", format!("{}\n{}\nr = chat[a, ask['hi there'], system = 'be brief']\nlog[r.text]\nlog[r.stop]\nlog[r.usage.input]\nlog[r.usage.output]", open, anthropic)),
        ("anthropic-tool", format!("{}\n{}\n{}", open, anthropic, tool_flow("a"))),
        ("anthropic-act", format!("{}\n{}\nd = act[a, ask['hi'], tools = [self_tool_@N]]\nlog[d.text]\nlog[d.stop]\nlog[d.rounds]\nlog[d.steps.how]", open, anthropic)),
        ("anthropic-follow", format!("{}\n{}\ns = follow[a, ask['one two']]\n{}\nlog[whole.text]\nlog[whole.stop]\nlog[whole.usage.output]", open, anthropic, follow("s"))),
        ("anthropic-follow-tool", format!("{}\n{}\ns = follow[a, ask['hi'], tools = [self_tool_@N]]\n{}\nlog[whole.stop]\nlog[whole.calls[0].id]\nlog[whole.calls[0].args.x]", open, anthropic, follow("s"))),
        ("anthropic-embed", format!("{}\n{}\nlog[embed[a, ['x']].reason]", open, anthropic)),
        ("refused", "self_p = env['CROW_SELF_LLM', '9']\nm = reach['openai://127.0.0.1:`self_p`/self-model?path=/refuse']\nr = chat[m, ask['hi']]\nlog[r.ok]\nlog[r.reason]".into()),
        ("no-json", "self_p = env['CROW_SELF_LLM', '9']\nm = reach['openai://127.0.0.1:`self_p`/self-model?path=/broken']\nr = chat[m, ask['hi']]\nlog[r.ok]\nlog[r.reason]".into()),
        ("nobody", "m = reach['openai://127.0.0.1:9/self-model']\nr = chat[m, ask['hi']]\nlog[r.ok]\nlog[r.kind]".into()),
    ];
    // (a tool the model may call: the compiler writes its schema from its
    // comment and its parameter, which its body reads as text — one only
    // handed to the model is typed by nothing else)
    let tool = ["--- says its argument back", "fn self_tool_@N[x]", "    return 'echo `trim[x]`'"];
    let mut cases: Vec<Case> = pure
        .into_iter()
        .map(|(name, body)| Case::new(format!("llm/{}", name), &[], body.lines().map(str::to_string).collect()).referring(format!("llm:{}", name), &[]).with_import("llm"))
        .collect();
    for (name, body) in live {
        let mut c = Case::new(format!("llm-live/{}", name), &[], body.lines().map(str::to_string).collect()).live().referring(format!("llm:{}", name), &[]).with_import("llm");
        if body.contains("self_tool_@N") {
            c.top = tool.iter().map(|l| l.to_string()).collect();
        }
        cases.push(c);
    }
    cases
}
