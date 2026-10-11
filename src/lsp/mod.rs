/// `crow lsp` — the Crow language server.
///
/// Speaks the Language Server Protocol over stdio. One in-memory document
/// store; every change re-runs the same pipeline the compiler runs and pushes
/// diagnostics. Features answered from the last good analysis: completion,
/// hover, go-to-definition, document symbols. Database hazards from db_check
/// appear as warnings when the workspace crowfile sets `db = '…'`.

pub mod analysis;
pub mod features;

use lsp_server::{Connection, Message, Notification as ServerNotification, Response};
use lsp_types::notification::{
    DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument, Notification as _,
    PublishDiagnostics,
};
use lsp_types::request::{
    Completion, DocumentSymbolRequest, GotoDefinition, HoverRequest, Request as _,
};
use lsp_types::{
    CompletionResponse, DocumentSymbolResponse, GotoDefinitionResponse, Hover, HoverContents,
    InitializeParams, MarkedString, OneOf, PublishDiagnosticsParams, ServerCapabilities,
    TextDocumentSyncCapability, TextDocumentSyncKind, Uri,
};
use std::collections::HashMap;
use std::path::PathBuf;

pub fn run_server() -> Result<(), Box<dyn std::error::Error + Sync + Send>> {
    // the server answers from the program's text and what its databases
    // answered last (`.crow/schema`), never a database asked on every
    // keystroke: a part validated alone had asked one
    // (Rust 2024: the environment is process-global; no thread runs yet)
    unsafe { std::env::set_var("CROW_NO_DESCRIBE", "1") };
    let binary = std::env::current_exe().ok().map(Binary::of);
    let version = match &binary {
        Some(b) => format!("{} {}", env!("CARGO_PKG_VERSION"), b.describe()),
        None => env!("CARGO_PKG_VERSION").to_string(),
    };
    // (the editor's output pane for the server says which crow answers)
    eprintln!("crow lsp: crow {}", version);
    let (connection, io_threads) = Connection::stdio();
    let capabilities = ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        completion_provider: Some(Default::default()),
        signature_help_provider: Some(lsp_types::SignatureHelpOptions {
            trigger_characters: Some(vec!["[".into(), ",".into()]),
            retrigger_characters: None,
            work_done_progress_options: Default::default(),
        }),
        hover_provider: Some(lsp_types::HoverProviderCapability::Simple(true)),
        definition_provider: Some(OneOf::Left(true)),
        references_provider: Some(OneOf::Left(true)),
        document_symbol_provider: Some(OneOf::Left(true)),
        code_action_provider: Some(lsp_types::CodeActionProviderCapability::Simple(true)),
        rename_provider: Some(OneOf::Right(lsp_types::RenameOptions { prepare_provider: Some(true), work_done_progress_options: Default::default() })),
        ..Default::default()
    };
    let (id, init_params) = connection.initialize_start()?;
    let result = lsp_types::InitializeResult {
        capabilities,
        server_info: Some(lsp_types::ServerInfo { name: "crow".into(), version: Some(version) }),
    };
    connection.initialize_finish(id, serde_json::to_value(result)?)?;
    if main_loop(&connection, init_params, binary.as_ref())? == Left::BinaryChanged {
        // the editor starts the server again, on the binary installed now
        // (the reader thread waits on a stdin the client keeps open: no join)
        eprintln!("crow lsp: the crow binary changed since this server started — stopping, so the editor starts the new one");
        std::process::exit(0);
    }
    // The writer thread ends when the last sender is dropped — drop the
    // connection before joining or the join deadlocks.
    drop(connection);
    io_threads.join()?;
    Ok(())
}

pub(crate) struct State {
    docs: HashMap<String, analysis::Analysis>,
    /// Database url from the workspace crowfile, resolved once at startup.
    db_url: Option<String>,
}

/// Why the loop ended: the client ended the session (a shutdown, or its
/// connection closed), or the server's binary changed under it.
#[derive(Debug, PartialEq)]
enum Left {
    ByClient,
    BinaryChanged,
}

/// The binary this server runs: its path, and the file's modified time and
/// length as the server started. An install replaces the file while an
/// editor's server keeps running the old one — it had served a two-day-old
/// compiler, refusing what the installed crow accepted — so the loop
/// watches the file and leaves when it is no longer the one it started
/// from; the editor starts the server again, on the new binary.
pub(crate) struct Binary {
    path: PathBuf,
    stamp: (Option<std::time::SystemTime>, u64),
}

impl Binary {
    pub(crate) fn of(path: PathBuf) -> Binary {
        let stamp = stamp(&path);
        Binary { path, stamp }
    }

    /// Whether the file is no longer the one the server started from:
    /// rewritten, replaced or gone. A rewrite still in progress is waited
    /// out, so the editor does not start a half-copied binary.
    pub(crate) fn changed(&self) -> bool {
        let mut now = stamp(&self.path);
        if now == self.stamp {
            return false;
        }
        for _ in 0..40 {
            std::thread::sleep(std::time::Duration::from_millis(250));
            let next = stamp(&self.path);
            if next == now {
                break;
            }
            now = next;
        }
        true
    }

    /// "at /usr/local/bin/crow, written Oct 10 16:32" (the file's modified
    /// time: the install, for an installed crow).
    fn describe(&self) -> String {
        let built = match self.stamp.0 {
            Some(t) => chrono::DateTime::<chrono::Local>::from(t).format("written %b %d %H:%M").to_string(),
            None => "written at an unknown time".to_string(),
        };
        format!("at {}, {}", self.path.display(), built)
    }
}

/// A file's modified time and length; none and zero when it cannot be read.
fn stamp(path: &std::path::Path) -> (Option<std::time::SystemTime>, u64) {
    std::fs::metadata(path).map(|m| (m.modified().ok(), m.len())).unwrap_or((None, 0))
}

fn main_loop(
    connection: &Connection,
    init_params: serde_json::Value,
    binary: Option<&Binary>,
) -> Result<Left, Box<dyn std::error::Error + Sync + Send>> {
    let init: InitializeParams = serde_json::from_value(init_params).unwrap_or_default();
    let root = init
        .workspace_folders
        .as_ref()
        .and_then(|f| f.first())
        .map(|f| PathBuf::from(f.uri.path().as_str()))
        .or_else(|| {
            // Older clients send only the deprecated rootUri.
            #[allow(deprecated)]
            let legacy = init.root_uri.as_ref();
            legacy.map(|u| PathBuf::from(u.path().as_str()))
        });
    let db_url = root.as_deref().and_then(crate::crowfile_db_url_at);

    let mut state = State {
        docs: HashMap::new(),
        db_url,
    };

    // messages waiting behind a document's change, read ahead while a later
    // change to the same document was looked for
    let mut waiting: std::collections::VecDeque<Message> = std::collections::VecDeque::new();
    loop {
        let msg = match waiting.pop_front() {
            Some(m) => m,
            None => match connection.receiver.recv_timeout(std::time::Duration::from_secs(2)) {
                Ok(m) => m,
                // (idle: the binary is looked at every two seconds)
                Err(e) if e.is_timeout() => {
                    if binary.is_some_and(Binary::changed) {
                        return Ok(Left::BinaryChanged);
                    }
                    continue;
                }
                Err(_) => break,
            },
        };
        // a message arriving after the binary changed is the new server's
        // to answer: the client opens its documents again when it starts
        if binary.is_some_and(Binary::changed) {
            return Ok(Left::BinaryChanged);
        }
        // a change another change to the same document has overtaken is not
        // analysed: typing into a file that takes seconds to analyse had
        // queued an analysis per keystroke
        let msg = match msg {
            Message::Notification(not) => Message::Notification(latest_change(connection, not, &mut waiting)),
            other => other,
        };
        match msg {
            Message::Request(req) => {
                if connection.handle_shutdown(&req)? {
                    return Ok(Left::ByClient);
                }
                let resp = handle_request(&state, req);
                connection.sender.send(Message::Response(resp))?;
            }
            Message::Notification(not) => {
                if let Some((uri, text)) = doc_event(not) {
                    match text {
                        Some(text) => {
                            let diags = open_or_change(&mut state, &uri, text);
                            publish(connection, &uri, diags)?;
                        }
                        None => {
                            state.docs.remove(uri.as_str());
                            publish(connection, &uri, Vec::new())?;
                        }
                    }
                }
            }
            Message::Response(_) => {}
        }
    }
    Ok(Left::ByClient)
}

/// A document's open or change, or the latest change to it already waiting
/// behind it; every other message waiting is kept, in order.
fn latest_change(connection: &Connection, not: ServerNotification, waiting: &mut std::collections::VecDeque<Message>) -> ServerNotification {
    let uri_of = |n: &ServerNotification| -> Option<String> {
        if n.method != DidChangeTextDocument::METHOD && n.method != DidOpenTextDocument::METHOD {
            return None;
        }
        n.params.get("textDocument")?.get("uri")?.as_str().map(str::to_string)
    };
    let Some(uri) = uri_of(&not) else { return not };
    let mut latest = not;
    while let Ok(next) = connection.receiver.try_recv() {
        match next {
            Message::Notification(n) if n.method == DidChangeTextDocument::METHOD && uri_of(&n).as_deref() == Some(uri.as_str()) => latest = n,
            other => {
                waiting.push_back(other);
                // (what follows a request is answered after it, from this
                // document's analysis)
                break;
            }
        }
    }
    latest
}

/// didOpen/didChange → (uri, Some(full text)); didClose → (uri, None).
fn doc_event(not: ServerNotification) -> Option<(Uri, Option<String>)> {
    match not.method.as_str() {
        DidOpenTextDocument::METHOD => {
            let p: lsp_types::DidOpenTextDocumentParams =
                serde_json::from_value(not.params).ok()?;
            Some((p.text_document.uri, Some(p.text_document.text)))
        }
        DidChangeTextDocument::METHOD => {
            let p: lsp_types::DidChangeTextDocumentParams =
                serde_json::from_value(not.params).ok()?;
            // FULL sync: the last change carries the entire document.
            let text = p.content_changes.into_iter().last()?.text;
            Some((p.text_document.uri, Some(text)))
        }
        DidCloseTextDocument::METHOD => {
            let p: lsp_types::DidCloseTextDocumentParams =
                serde_json::from_value(not.params).ok()?;
            Some((p.text_document.uri, None))
        }
        _ => None,
    }
}

fn open_or_change(state: &mut State, uri: &Uri, text: String) -> Vec<lsp_types::Diagnostic> {
    let path = PathBuf::from(uri.path().as_str());
    let dir = path.parent().map(|p| p.to_path_buf());
    let label = path
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_else(|| "file.co".into());
    let analysis = analysis::analyze(&text, dir.as_deref(), state.db_url.as_deref(), &label);
    let diags = analysis.diagnostics.clone();
    state.docs.insert(uri.as_str().to_string(), analysis);
    diags
}

fn publish(
    connection: &Connection,
    uri: &Uri,
    diagnostics: Vec<lsp_types::Diagnostic>,
) -> Result<(), Box<dyn std::error::Error + Sync + Send>> {
    let params = PublishDiagnosticsParams {
        uri: uri.clone(),
        diagnostics,
        version: None,
    };
    connection
        .sender
        .send(Message::Notification(ServerNotification::new(
            PublishDiagnostics::METHOD.to_string(),
            serde_json::to_value(params)?,
        )))?;
    Ok(())
}

fn handle_request(state: &State, req: lsp_server::Request) -> Response {
    match req.method.as_str() {
        Completion::METHOD => {
            let p: lsp_types::CompletionParams = match serde_json::from_value(req.params) {
                Ok(p) => p,
                Err(e) => return Response::new_err(req.id, -32602, e.to_string()),
            };
            let uri = p.text_document_position.text_document.uri;
            let items = state
                .docs
                .get(uri.as_str())
                .map(features::completions)
                .unwrap_or_default();
            Response::new_ok(req.id, CompletionResponse::Array(items))
        }
        lsp_types::request::SignatureHelpRequest::METHOD => {
            let p: lsp_types::SignatureHelpParams = match serde_json::from_value(req.params) {
                Ok(p) => p,
                Err(e) => return Response::new_err(req.id, -32602, e.to_string()),
            };
            let uri = p.text_document_position_params.text_document.uri;
            let pos = p.text_document_position_params.position;
            let help = state
                .docs
                .get(uri.as_str())
                .and_then(|a| features::signature_help(a, pos.line, pos.character));
            Response::new_ok(req.id, help)
        }
        HoverRequest::METHOD => {
            let p: lsp_types::HoverParams = match serde_json::from_value(req.params) {
                Ok(p) => p,
                Err(e) => return Response::new_err(req.id, -32602, e.to_string()),
            };
            let uri = p.text_document_position_params.text_document.uri;
            let pos = p.text_document_position_params.position;
            let hover = state
                .docs
                .get(uri.as_str())
                .and_then(|d| features::hover(d, pos))
                .map(|text| Hover {
                    contents: HoverContents::Scalar(MarkedString::String(text)),
                    range: None,
                });
            Response::new_ok(req.id, hover)
        }
        GotoDefinition::METHOD => {
            let p: lsp_types::GotoDefinitionParams = match serde_json::from_value(req.params) {
                Ok(p) => p,
                Err(e) => return Response::new_err(req.id, -32602, e.to_string()),
            };
            let uri = p.text_document_position_params.text_document.uri;
            let pos = p.text_document_position_params.position;
            let loc = state
                .docs
                .get(uri.as_str())
                .and_then(|d| features::definition(d, pos, &uri))
                .map(GotoDefinitionResponse::Scalar);
            Response::new_ok(req.id, loc)
        }
        lsp_types::request::References::METHOD => {
            let p: lsp_types::ReferenceParams = match serde_json::from_value(req.params) {
                Ok(p) => p,
                Err(e) => return Response::new_err(req.id, -32602, e.to_string()),
            };
            let uri = p.text_document_position.text_document.uri;
            let pos = p.text_document_position.position;
            let refs = state.docs.get(uri.as_str()).map(|d| features::references(d, pos, &uri)).unwrap_or_default();
            Response::new_ok(req.id, refs)
        }
        DocumentSymbolRequest::METHOD => {
            let p: lsp_types::DocumentSymbolParams = match serde_json::from_value(req.params) {
                Ok(p) => p,
                Err(e) => return Response::new_err(req.id, -32602, e.to_string()),
            };
            let syms = state
                .docs
                .get(p.text_document.uri.as_str())
                .map(features::document_symbols)
                .map(DocumentSymbolResponse::Nested)
                .unwrap_or(DocumentSymbolResponse::Nested(Vec::new()));
            Response::new_ok(req.id, syms)
        }
        lsp_types::request::CodeActionRequest::METHOD => {
            let p: lsp_types::CodeActionParams = match serde_json::from_value(req.params) {
                Ok(p) => p,
                Err(e) => return Response::new_err(req.id, -32602, e.to_string()),
            };
            let uri = p.text_document.uri;
            let actions = state.docs.get(uri.as_str()).map(|a| features::code_actions(a, &uri, &p.context.diagnostics)).unwrap_or_default();
            Response::new_ok(req.id, actions)
        }
        lsp_types::request::PrepareRenameRequest::METHOD => {
            let p: lsp_types::TextDocumentPositionParams = match serde_json::from_value(req.params) {
                Ok(p) => p,
                Err(e) => return Response::new_err(req.id, -32602, e.to_string()),
            };
            let answer = state.docs.get(p.text_document.uri.as_str()).and_then(|a| features::prepare_rename(a, p.position));
            match answer {
                Some(Ok(range)) => Response::new_ok(req.id, lsp_types::PrepareRenameResponse::Range(range)),
                Some(Err(why)) => Response::new_err(req.id, -32803, why), // RequestFailed
                None => Response::new_ok(req.id, serde_json::Value::Null),
            }
        }
        lsp_types::request::Rename::METHOD => {
            let p: lsp_types::RenameParams = match serde_json::from_value(req.params) {
                Ok(p) => p,
                Err(e) => return Response::new_err(req.id, -32602, e.to_string()),
            };
            let uri = p.text_document_position.text_document.uri;
            let pos = p.text_document_position.position;
            match state.docs.get(uri.as_str()).map(|a| features::rename(a, pos, &p.new_name, &uri)) {
                Some(Ok(edit)) => Response::new_ok(req.id, edit),
                Some(Err(why)) => Response::new_err(req.id, -32803, why),
                None => Response::new_ok(req.id, serde_json::Value::Null),
            }
        }
        _ => Response::new_err(
            req.id,
            -32601, // MethodNotFound
            format!("unhandled method '{}'", req.method),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lsp_server::{Notification as N, Request as R, RequestId};

    /// Full protocol smoke test over an in-memory connection: initialize,
    /// open a broken document, receive diagnostics, hover a symbol, shutdown.
    #[test]
    fn protocol_session_end_to_end() {
        let (server_conn, client_conn) = Connection::memory();

        let server = std::thread::spawn(move || {
            let caps = ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                hover_provider: Some(lsp_types::HoverProviderCapability::Simple(true)),
                ..Default::default()
            };
            let init = server_conn
                .initialize(serde_json::to_value(caps).unwrap())
                .unwrap();
            main_loop(&server_conn, init, None).unwrap();
        });

        // -- handshake
        client_conn
            .sender
            .send(Message::Request(R::new(
                RequestId::from(1),
                "initialize".into(),
                serde_json::json!({"capabilities": {}}),
            )))
            .unwrap();
        let _init_resp = client_conn.receiver.recv().unwrap();
        client_conn
            .sender
            .send(Message::Notification(N::new(
                "initialized".into(),
                serde_json::json!({}),
            )))
            .unwrap();

        // -- open a file with one broken line and one good fn
        let text = "fn add[a, b]\n    return a + b\n\nfn = = broken\nx = add[1, 2]\n";
        client_conn
            .sender
            .send(Message::Notification(N::new(
                DidOpenTextDocument::METHOD.into(),
                serde_json::json!({"textDocument": {
                    "uri": "file:///demo.co", "languageId": "crow",
                    "version": 1, "text": text}}),
            )))
            .unwrap();

        // -- diagnostics arrive, flagging the broken line (0-based line 3)
        let msg = client_conn.receiver.recv().unwrap();
        let Message::Notification(n) = msg else {
            panic!("expected publishDiagnostics, got {:?}", msg)
        };
        assert_eq!(n.method, PublishDiagnostics::METHOD);
        let p: PublishDiagnosticsParams = serde_json::from_value(n.params).unwrap();
        assert!(!p.diagnostics.is_empty());
        assert!(p.diagnostics.iter().any(|d| d.range.start.line == 3));

        // -- hover on `add` at the call site still works despite the break
        client_conn
            .sender
            .send(Message::Request(R::new(
                RequestId::from(2),
                HoverRequest::METHOD.into(),
                serde_json::json!({
                    "textDocument": {"uri": "file:///demo.co"},
                    "position": {"line": 4, "character": 5}}),
            )))
            .unwrap();
        let msg = client_conn.receiver.recv().unwrap();
        let Message::Response(resp) = msg else {
            panic!("expected hover response")
        };
        let hover = serde_json::to_value(&resp).unwrap().to_string();
        assert!(
            hover.contains("fn add"),
            "hover should show the signature: {}",
            hover
        );

        // -- shutdown
        client_conn
            .sender
            .send(Message::Request(R::new(
                RequestId::from(3),
                "shutdown".into(),
                serde_json::Value::Null,
            )))
            .unwrap();
        let _ = client_conn.receiver.recv().unwrap();
        client_conn
            .sender
            .send(Message::Notification(N::new("exit".into(), serde_json::Value::Null)))
            .unwrap();
        server.join().unwrap();
    }

    /// The server leaves when its binary is replaced, as an install
    /// replaces it, so the editor starts the new one; a message arriving
    /// after the change is not answered by the old server (it had kept
    /// answering, two days after the install).
    #[test]
    fn the_server_leaves_when_its_binary_changes() {
        let dir = std::env::temp_dir().join(format!("crow_lsp_binary_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("crow");
        std::fs::write(&exe, b"the binary as installed").unwrap();
        let binary = Binary::of(exe.clone());
        assert!(!binary.changed(), "untouched, it is the binary the server started from");

        let (server_conn, client_conn) = Connection::memory();
        let server = std::thread::spawn(move || {
            let init = server_conn.initialize(serde_json::json!({})).unwrap();
            main_loop(&server_conn, init, Some(&binary)).unwrap()
        });
        client_conn.sender.send(Message::Request(R::new(RequestId::from(1), "initialize".into(), serde_json::json!({"capabilities": {}})))).unwrap();
        let _ = client_conn.receiver.recv().unwrap();
        client_conn.sender.send(Message::Notification(N::new("initialized".into(), serde_json::json!({})))).unwrap();
        client_conn
            .sender
            .send(Message::Notification(N::new(
                DidOpenTextDocument::METHOD.into(),
                serde_json::json!({"textDocument": {"uri": "file:///demo.co", "languageId": "crow", "version": 1, "text": "x = 1\n"}}),
            )))
            .unwrap();
        let Message::Notification(n) = client_conn.receiver.recv().unwrap() else { panic!("expected publishDiagnostics") };
        assert_eq!(n.method, PublishDiagnostics::METHOD);

        // the install: a rebuilt binary in the file's place
        std::fs::write(&exe, b"the binary as installed, then rebuilt").unwrap();
        client_conn
            .sender
            .send(Message::Request(R::new(
                RequestId::from(2),
                HoverRequest::METHOD.into(),
                serde_json::json!({"textDocument": {"uri": "file:///demo.co"}, "position": {"line": 0, "character": 0}}),
            )))
            .unwrap();
        assert_eq!(server.join().unwrap(), Left::BinaryChanged);
        // (the old server answered nothing: its connection closed instead)
        assert!(client_conn.receiver.recv().is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A binary rewritten to the same length is noticed by its modified
    /// time, a binary gone is changed, and one stamped anew is current
    #[test]
    fn a_binary_is_known_by_its_modified_time_and_length() {
        let dir = std::env::temp_dir().join(format!("crow_lsp_stamp_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("crow");
        std::fs::write(&exe, b"a binary").unwrap();
        let binary = Binary::of(exe.clone());
        assert!(!binary.changed());
        assert!(binary.describe().starts_with(&format!("at {}, written ", exe.display())), "{}", binary.describe());
        // (the same bytes, written later: a filesystem counting whole
        // seconds would not tell them apart, so the time is set outright)
        std::fs::write(&exe, b"a binary").unwrap();
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
        std::fs::File::options().write(true).open(&exe).unwrap().set_modified(later).unwrap();
        assert!(binary.changed(), "rewritten to the same length, it is a different binary");
        let binary = Binary::of(exe.clone());
        assert!(!binary.changed(), "stamped anew, it is current");
        std::fs::remove_file(&exe).unwrap();
        assert!(binary.changed(), "gone, it is not the binary the server started from");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
