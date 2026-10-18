// Crow's VS Code extension: `crow lsp` started and spoken to over stdio,
// with VS Code's own API and the editor's built-in modules alone. It needs
// no npm package: `vscode-languageclient` had been its one dependency, and
// with it npm and Node.js in Crow's own release. All intelligence lives in
// the server; this file frames its messages (LSP's base protocol: a
// Content-Length header, then the JSON), keeps the open documents in step,
// shows what the server publishes and asks it what the editor asks — and
// starts it again when it leaves (it does when its binary is replaced, so
// the editor runs the crow installed now).
"use strict";

const vscode = require("vscode");
const { spawn } = require("child_process");

const SELECTOR = { scheme: "file", language: "crow" };

// ── the wire ──────────────────────────────────────────────────────────

/** The messages a byte stream carries, each handed on whole as it ends. */
class Framer {
  constructor(onMessage) {
    this.buffer = Buffer.alloc(0);
    this.onMessage = onMessage;
  }

  push(chunk) {
    this.buffer = Buffer.concat([this.buffer, chunk]);
    for (;;) {
      const end = this.buffer.indexOf("\r\n\r\n");
      if (end < 0) return;
      const head = this.buffer.slice(0, end).toString("ascii");
      const found = /Content-Length:\s*(\d+)/i.exec(head);
      if (!found) {
        this.buffer = this.buffer.slice(end + 4);
        continue;
      }
      const length = parseInt(found[1], 10);
      if (this.buffer.length < end + 4 + length) return;
      const body = this.buffer.slice(end + 4, end + 4 + length).toString("utf8");
      this.buffer = this.buffer.slice(end + 4 + length);
      let message;
      try {
        message = JSON.parse(body);
      } catch (_) {
        continue;
      }
      this.onMessage(message);
    }
  }
}

function framed(message) {
  const body = Buffer.from(JSON.stringify(message), "utf8");
  return Buffer.concat([Buffer.from(`Content-Length: ${body.length}\r\n\r\n`, "ascii"), body]);
}

// ── between the editor's values and the protocol's ────────────────────

const toPosition = (p) => ({ line: p.line, character: p.character });
const fromPosition = (p) => new vscode.Position(p.line, p.character);
const fromRange = (r) => new vscode.Range(r.start.line, r.start.character, r.end.line, r.end.character);
const toRange = (r) => ({ start: toPosition(r.start), end: toPosition(r.end) });

function fromMarkup(content) {
  if (content === undefined || content === null) return undefined;
  if (typeof content === "string") return new vscode.MarkdownString(content);
  if (Array.isArray(content)) {
    const md = new vscode.MarkdownString();
    for (const part of content) appendMarked(md, part);
    return md;
  }
  if (content.kind === "markdown") return new vscode.MarkdownString(content.value);
  if (content.kind === "plaintext") return new vscode.MarkdownString().appendText(content.value);
  const md = new vscode.MarkdownString();
  appendMarked(md, content);
  return md;
}

function appendMarked(md, part) {
  if (typeof part === "string") md.appendMarkdown(part + "\n\n");
  else if (part && part.language !== undefined) md.appendCodeblock(part.value, part.language);
  else if (part && part.value !== undefined) md.appendMarkdown(part.value + "\n\n");
}

const SEVERITY = [undefined, vscode.DiagnosticSeverity.Error, vscode.DiagnosticSeverity.Warning, vscode.DiagnosticSeverity.Information, vscode.DiagnosticSeverity.Hint];

function fromDiagnostic(d) {
  const diagnostic = new vscode.Diagnostic(fromRange(d.range), d.message, SEVERITY[d.severity] ?? vscode.DiagnosticSeverity.Error);
  if (d.source) diagnostic.source = d.source;
  if (d.code !== undefined && d.code !== null) diagnostic.code = d.code;
  if (Array.isArray(d.relatedInformation)) {
    diagnostic.relatedInformation = d.relatedInformation.map(
      (r) => new vscode.DiagnosticRelatedInformation(new vscode.Location(vscode.Uri.parse(r.location.uri), fromRange(r.location.range)), r.message)
    );
  }
  return diagnostic;
}

function toDiagnostic(d) {
  const severity = { [vscode.DiagnosticSeverity.Error]: 1, [vscode.DiagnosticSeverity.Warning]: 2, [vscode.DiagnosticSeverity.Information]: 3, [vscode.DiagnosticSeverity.Hint]: 4 }[d.severity];
  const out = { range: toRange(d.range), message: d.message, severity };
  if (d.source) out.source = d.source;
  if (d.code !== undefined) out.code = typeof d.code === "object" && d.code !== null ? d.code.value : d.code;
  return out;
}

function fromLocation(l) {
  if (l.targetUri !== undefined) {
    return { originSelectionRange: l.originSelectionRange && fromRange(l.originSelectionRange), targetUri: vscode.Uri.parse(l.targetUri), targetRange: fromRange(l.targetRange), targetSelectionRange: fromRange(l.targetSelectionRange || l.targetRange) };
  }
  return new vscode.Location(vscode.Uri.parse(l.uri), fromRange(l.range));
}

function fromWorkspaceEdit(e) {
  const edit = new vscode.WorkspaceEdit();
  if (!e) return edit;
  if (Array.isArray(e.documentChanges)) {
    for (const change of e.documentChanges) {
      if (change.textDocument && Array.isArray(change.edits)) {
        const uri = vscode.Uri.parse(change.textDocument.uri);
        for (const t of change.edits) edit.replace(uri, fromRange(t.range), t.newText);
      }
    }
  } else if (e.changes) {
    for (const [uri, edits] of Object.entries(e.changes)) {
      for (const t of edits) edit.replace(vscode.Uri.parse(uri), fromRange(t.range), t.newText);
    }
  }
  return edit;
}

function fromCompletion(c) {
  const item = new vscode.CompletionItem(c.label, c.kind ? c.kind - 1 : undefined);
  if (c.detail) item.detail = c.detail;
  if (c.documentation) item.documentation = fromMarkup(c.documentation);
  if (c.sortText) item.sortText = c.sortText;
  if (c.filterText) item.filterText = c.filterText;
  if (c.preselect) item.preselect = true;
  const text = c.textEdit ? c.textEdit.newText : c.insertText;
  if (text !== undefined) item.insertText = c.insertTextFormat === 2 ? new vscode.SnippetString(text) : text;
  if (c.textEdit) {
    item.range = c.textEdit.range ? fromRange(c.textEdit.range) : { inserting: fromRange(c.textEdit.insert), replacing: fromRange(c.textEdit.replace) };
  }
  if (Array.isArray(c.additionalTextEdits)) item.additionalTextEdits = c.additionalTextEdits.map((t) => vscode.TextEdit.replace(fromRange(t.range), t.newText));
  return item;
}

function fromSymbol(s, uri) {
  if (s.location) {
    return new vscode.SymbolInformation(s.name, s.kind - 1, s.containerName || "", new vscode.Location(vscode.Uri.parse(s.location.uri), fromRange(s.location.range)));
  }
  const symbol = new vscode.DocumentSymbol(s.name, s.detail || "", s.kind - 1, fromRange(s.range), fromRange(s.selectionRange || s.range));
  symbol.children = (s.children || []).map((c) => fromSymbol(c, uri));
  return symbol;
}

function fromSignatures(h) {
  const help = new vscode.SignatureHelp();
  help.activeSignature = h.activeSignature || 0;
  help.activeParameter = h.activeParameter || 0;
  help.signatures = (h.signatures || []).map((s) => {
    const sig = new vscode.SignatureInformation(s.label, fromMarkup(s.documentation));
    sig.parameters = (s.parameters || []).map((p) => new vscode.ParameterInformation(p.label, fromMarkup(p.documentation)));
    if (s.activeParameter !== undefined) sig.activeParameter = s.activeParameter;
    return sig;
  });
  return help;
}

// ── the server ────────────────────────────────────────────────────────

class Server {
  constructor(output, diagnostics) {
    this.output = output;
    this.diagnostics = diagnostics;
    this.child = null;
    this.next = 1;
    this.pending = new Map();
    this.ready = Promise.resolve(false);
    this.up = false;
    this.stopping = false;
    this.starts = [];
  }

  start() {
    const command = vscode.workspace.getConfiguration("crow").get("serverPath", "crow");
    this.output.appendLine(`starting ${command} lsp`);
    let child;
    try {
      child = spawn(command, ["lsp"], { stdio: ["pipe", "pipe", "pipe"] });
    } catch (e) {
      this.output.appendLine(`cannot start ${command}: ${e.message}`);
      return;
    }
    this.child = child;
    this.up = false;
    this.starts.push(Date.now());
    const framer = new Framer((m) => this.receive(m));
    child.stdout.on("data", (chunk) => framer.push(chunk));
    child.stderr.on("data", (chunk) => this.output.append(chunk.toString()));
    child.on("error", (e) => this.output.appendLine(`the server could not run: ${e.message} — set crow.serverPath to the crow binary`));
    child.on("exit", (code, signal) => this.left(child, code, signal));
    this.ready = this.initialize(child);
  }

  async initialize(child) {
    const folders = vscode.workspace.workspaceFolders || [];
    try {
      await this.request("initialize", {
        processId: process.pid,
        clientInfo: { name: "Crow for VS Code" },
        rootUri: folders.length ? folders[0].uri.toString() : null,
        workspaceFolders: folders.map((f) => ({ uri: f.uri.toString(), name: f.name })),
        capabilities: {
          general: { positionEncodings: ["utf-16"] },
          workspace: { workspaceEdit: { documentChanges: true } },
          textDocument: {
            synchronization: { dynamicRegistration: false, didSave: false },
            publishDiagnostics: { relatedInformation: true },
            completion: { completionItem: { snippetSupport: true, documentationFormat: ["markdown", "plaintext"] } },
            hover: { contentFormat: ["markdown", "plaintext"] },
            signatureHelp: { signatureInformation: { documentationFormat: ["markdown", "plaintext"], parameterInformation: { labelOffsetSupport: true } } },
            definition: { linkSupport: true },
            references: {},
            documentSymbol: { hierarchicalDocumentSymbolSupport: true },
            codeAction: { codeActionLiteralSupport: { codeActionKind: { valueSet: ["", "quickfix", "refactor", "source"] } } },
            rename: { prepareSupport: true },
          },
        },
      });
    } catch (e) {
      this.output.appendLine(`the server did not start: ${e.message}`);
      return false;
    }
    if (this.child !== child) return false;
    this.notify("initialized", {});
    // (every document open now, as it stands: one opened while the server
    // started is told here, and its changes after)
    this.up = true;
    for (const doc of vscode.workspace.textDocuments) this.opened(doc);
    return true;
  }

  /** The server left: what waited on it is answered with nothing, and it is
   * started again unless the editor is closing it — at most five times in
   * three minutes, after which it says so and waits. */
  left(child, code, signal) {
    if (this.child !== child) return;
    this.child = null;
    this.up = false;
    for (const p of this.pending.values()) p.resolve(null);
    this.pending.clear();
    this.diagnostics.clear();
    if (this.stopping) return;
    this.output.appendLine(`the server left (${signal || `exit ${code}`})`);
    const recent = this.starts.filter((t) => Date.now() - t < 180000);
    this.starts = recent;
    if (recent.length >= 5) {
      vscode.window.showErrorMessage("The Crow language server stopped five times in three minutes. Run `crow lsp` in a terminal to see why, then reload the window.");
      return;
    }
    setTimeout(() => this.start(), 300);
  }

  send(message) {
    if (this.child && this.child.stdin.writable) this.child.stdin.write(framed(message));
  }

  notify(method, params) {
    this.send({ jsonrpc: "2.0", method, params });
  }

  request(method, params, token) {
    if (!this.child) return Promise.resolve(null);
    const id = this.next++;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.send({ jsonrpc: "2.0", id, method, params });
      if (token) {
        token.onCancellationRequested(() => {
          if (this.pending.delete(id)) {
            this.notify("$/cancelRequest", { id });
            resolve(null);
          }
        });
      }
    });
  }

  /** A request once the server is up, or nothing when it is not. */
  async ask(method, params, token) {
    if (!(await this.ready)) return null;
    try {
      return await this.request(method, params, token);
    } catch (e) {
      this.output.appendLine(`${method}: ${e.message}`);
      return null;
    }
  }

  receive(m) {
    if (m.id !== undefined && m.method === undefined) {
      const p = this.pending.get(m.id);
      if (!p) return;
      this.pending.delete(m.id);
      if (m.error) p.reject(new Error(m.error.message));
      else p.resolve(m.result);
      return;
    }
    if (m.method === "textDocument/publishDiagnostics") {
      this.diagnostics.set(vscode.Uri.parse(m.params.uri), (m.params.diagnostics || []).map(fromDiagnostic));
      return;
    }
    if (m.method === "window/logMessage" || m.method === "window/showMessage") {
      this.output.appendLine(m.params.message);
      return;
    }
    // a request of the server's: none is asked of this client, so each is
    // answered with nothing
    if (m.id !== undefined) this.send({ jsonrpc: "2.0", id: m.id, result: null });
  }

  // the open documents, kept in step (the whole text each time: the server
  // syncs in full)
  opened(doc) {
    if (doc.languageId !== "crow" || doc.uri.scheme !== "file" || !this.up) return;
    this.notify("textDocument/didOpen", { textDocument: { uri: doc.uri.toString(), languageId: "crow", version: doc.version, text: doc.getText() } });
  }

  changed(event) {
    const doc = event.document;
    if (doc.languageId !== "crow" || doc.uri.scheme !== "file" || !this.up) return;
    this.notify("textDocument/didChange", { textDocument: { uri: doc.uri.toString(), version: doc.version }, contentChanges: [{ text: doc.getText() }] });
  }

  closed(doc) {
    if (doc.languageId !== "crow" || doc.uri.scheme !== "file") return;
    this.diagnostics.delete(doc.uri);
    if (this.up) this.notify("textDocument/didClose", { textDocument: { uri: doc.uri.toString() } });
  }

  async stop() {
    this.stopping = true;
    const child = this.child;
    if (!child) return;
    try {
      await Promise.race([this.request("shutdown", null), new Promise((r) => setTimeout(r, 1000))]);
      this.notify("exit", null);
    } catch (_) {
      // (it is going either way)
    }
    setTimeout(() => child.kill(), 500);
  }
}

// ── the editor's side ─────────────────────────────────────────────────

let server;

function activate(context) {
  const output = vscode.window.createOutputChannel("Crow Language Server");
  const diagnostics = vscode.languages.createDiagnosticCollection("crow");
  server = new Server(output, diagnostics);
  const at = (doc, position) => ({ textDocument: { uri: doc.uri.toString() }, position: toPosition(position) });
  context.subscriptions.push(
    output,
    diagnostics,
    vscode.workspace.onDidOpenTextDocument((doc) => server.opened(doc)),
    vscode.workspace.onDidChangeTextDocument((event) => server.changed(event)),
    vscode.workspace.onDidCloseTextDocument((doc) => server.closed(doc)),
    vscode.workspace.onDidChangeConfiguration((event) => {
      if (event.affectsConfiguration("crow.serverPath") && server.child) server.child.kill();
    }),
    vscode.languages.registerCompletionItemProvider(SELECTOR, {
      async provideCompletionItems(doc, position, token) {
        const r = await server.ask("textDocument/completion", at(doc, position), token);
        if (!r) return undefined;
        const items = (Array.isArray(r) ? r : r.items || []).map(fromCompletion);
        return new vscode.CompletionList(items, !Array.isArray(r) && !!r.isIncomplete);
      },
    }),
    vscode.languages.registerSignatureHelpProvider(SELECTOR, {
      async provideSignatureHelp(doc, position, token) {
        const r = await server.ask("textDocument/signatureHelp", at(doc, position), token);
        return r ? fromSignatures(r) : undefined;
      },
    }, "[", ","),
    vscode.languages.registerHoverProvider(SELECTOR, {
      async provideHover(doc, position, token) {
        const r = await server.ask("textDocument/hover", at(doc, position), token);
        if (!r || !r.contents) return undefined;
        return new vscode.Hover(fromMarkup(r.contents), r.range ? fromRange(r.range) : undefined);
      },
    }),
    vscode.languages.registerDefinitionProvider(SELECTOR, {
      async provideDefinition(doc, position, token) {
        const r = await server.ask("textDocument/definition", at(doc, position), token);
        if (!r) return undefined;
        return Array.isArray(r) ? r.map(fromLocation) : fromLocation(r);
      },
    }),
    vscode.languages.registerReferenceProvider(SELECTOR, {
      async provideReferences(doc, position, context, token) {
        const r = await server.ask("textDocument/references", { ...at(doc, position), context: { includeDeclaration: context.includeDeclaration } }, token);
        return r ? r.map(fromLocation) : undefined;
      },
    }),
    vscode.languages.registerDocumentSymbolProvider(SELECTOR, {
      async provideDocumentSymbols(doc, token) {
        const r = await server.ask("textDocument/documentSymbol", { textDocument: { uri: doc.uri.toString() } }, token);
        return r ? r.map((s) => fromSymbol(s, doc.uri)) : undefined;
      },
    }),
    vscode.languages.registerCodeActionsProvider(SELECTOR, {
      async provideCodeActions(doc, range, context, token) {
        const r = await server.ask("textDocument/codeAction", { textDocument: { uri: doc.uri.toString() }, range: toRange(range), context: { diagnostics: context.diagnostics.map(toDiagnostic) } }, token);
        if (!r) return undefined;
        return r.map((a) => {
          if (a.command !== undefined && typeof a.command === "string") {
            const action = new vscode.CodeAction(a.title);
            action.command = { title: a.title, command: a.command, arguments: a.arguments };
            return action;
          }
          const action = new vscode.CodeAction(a.title, a.kind ? vscode.CodeActionKind.Empty.append(a.kind) : undefined);
          if (a.edit) action.edit = fromWorkspaceEdit(a.edit);
          if (a.diagnostics) action.diagnostics = a.diagnostics.map(fromDiagnostic);
          if (a.isPreferred) action.isPreferred = true;
          return action;
        });
      },
    }, { providedCodeActionKinds: [vscode.CodeActionKind.QuickFix] }),
    vscode.languages.registerRenameProvider(SELECTOR, {
      async prepareRename(doc, position, token) {
        const r = await server.ask("textDocument/prepareRename", at(doc, position), token);
        if (!r) throw new Error("This name cannot be renamed.");
        if (r.defaultBehavior) return undefined;
        if (r.range) return { range: fromRange(r.range), placeholder: r.placeholder };
        return fromRange(r);
      },
      async provideRenameEdits(doc, position, newName, token) {
        const r = await server.ask("textDocument/rename", { ...at(doc, position), newName }, token);
        return fromWorkspaceEdit(r);
      },
    })
  );
  server.start();
  // (what the extension's own tests read: the server's state)
  return { server };
}

function deactivate() {
  return server ? server.stop() : undefined;
}

module.exports = { activate, deactivate, Framer, framed };
