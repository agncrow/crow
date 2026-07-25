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
    let (connection, io_threads) = Connection::stdio();
    let capabilities = ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        completion_provider: Some(Default::default()),
        hover_provider: Some(lsp_types::HoverProviderCapability::Simple(true)),
        definition_provider: Some(OneOf::Left(true)),
        document_symbol_provider: Some(OneOf::Left(true)),
        ..Default::default()
    };
    let init_params = connection.initialize(serde_json::to_value(capabilities)?)?;
    main_loop(&connection, init_params)?;
    // The writer thread ends when the last sender is dropped — drop the
    // connection before joining or the join deadlocks.
    drop(connection);
    io_threads.join()?;
    Ok(())
}

struct Doc {
    text: String,
    analysis: analysis::Analysis,
}

pub(crate) struct State {
    docs: HashMap<String, Doc>,
    /// Database url from the workspace crowfile, resolved once at startup.
    db_url: Option<String>,
}

fn main_loop(
    connection: &Connection,
    init_params: serde_json::Value,
) -> Result<(), Box<dyn std::error::Error + Sync + Send>> {
    let init: InitializeParams = serde_json::from_value(init_params).unwrap_or_default();
    let root = init
        .root_uri
        .as_ref()
        .map(|u| PathBuf::from(u.path().as_str()));
    let db_url = root.as_deref().and_then(crate::crowfile_db_url_at);

    let mut state = State {
        docs: HashMap::new(),
        db_url,
    };

    for msg in &connection.receiver {
        match msg {
            Message::Request(req) => {
                if connection.handle_shutdown(&req)? {
                    return Ok(());
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
    Ok(())
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
        .unwrap_or_else(|| "file.crow".into());
    let analysis = analysis::analyze(&text, dir.as_deref(), state.db_url.as_deref(), &label);
    let diags = analysis.diagnostics.clone();
    state
        .docs
        .insert(uri.as_str().to_string(), Doc { text, analysis });
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
                .map(|d| features::completions(&d.analysis))
                .unwrap_or_default();
            Response::new_ok(req.id, CompletionResponse::Array(items))
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
                .and_then(|d| features::hover(&d.analysis, pos))
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
                .and_then(|d| features::definition(&d.analysis, pos, &uri))
                .map(GotoDefinitionResponse::Scalar);
            Response::new_ok(req.id, loc)
        }
        DocumentSymbolRequest::METHOD => {
            let p: lsp_types::DocumentSymbolParams = match serde_json::from_value(req.params) {
                Ok(p) => p,
                Err(e) => return Response::new_err(req.id, -32602, e.to_string()),
            };
            let syms = state
                .docs
                .get(p.text_document.uri.as_str())
                .map(|d| features::document_symbols(&d.analysis))
                .map(DocumentSymbolResponse::Nested)
                .unwrap_or(DocumentSymbolResponse::Nested(Vec::new()));
            Response::new_ok(req.id, syms)
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
            main_loop(&server_conn, init).unwrap();
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
                    "uri": "file:///demo.crow", "languageId": "crow",
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
                    "textDocument": {"uri": "file:///demo.crow"},
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
}
