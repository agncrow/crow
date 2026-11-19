// The extension driven inside VS Code (editors/vscode/test.sh): every
// feature it relays — diagnostics, hover, completion, definition, symbols,
// references, signature help, rename, code actions, each name's kind —
// then the server killed and started again, the document told again. What
// it saw goes to CROW_EXT_TEST_OUT as JSON; any step that fails fails the
// run.
const vscode = require("vscode");
const fs = require("fs");
const OUT = process.env.CROW_EXT_TEST_OUT;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
async function until(what, f, ms = 30000) {
  const end = Date.now() + ms;
  for (;;) {
    const v = await f();
    if (v) return v;
    if (Date.now() > end) throw new Error("timed out waiting for " + what);
    await sleep(200);
  }
}
exports.run = async function () {
  const r = {};
  try {
    const ext = vscode.extensions.getExtension("crow-lang.crow-lang");
    const api = await ext.activate();
    const uri = vscode.Uri.file(process.env.CROW_EXT_TEST_WS + "/main.co");
    const doc = await vscode.workspace.openTextDocument(uri);
    await vscode.window.showTextDocument(doc);
    const diags = await until("diagnostics", () => { const d = vscode.languages.getDiagnostics(uri); return d.length ? d : null; });
    r.diagnostics = diags.map((d) => [d.range.start.line, d.message]);
    const hover = await vscode.commands.executeCommand("vscode.executeHoverProvider", uri, new vscode.Position(3, 5));
    r.hover = hover.map((h) => h.contents.map((c) => c.value || String(c)).join(" ")).join(" | ").slice(0, 200);
    const comp = await vscode.commands.executeCommand("vscode.executeCompletionItemProvider", uri, new vscode.Position(3, 4));
    r.completions = comp.items.length;
    r.completion_has_double = comp.items.some((i) => (i.label.label || i.label) === "double");
    const def = await vscode.commands.executeCommand("vscode.executeDefinitionProvider", uri, new vscode.Position(3, 6));
    r.definition = def.map((d) => (d.range || d.targetRange).start.line);
    const syms = await vscode.commands.executeCommand("vscode.executeDocumentSymbolProvider", uri);
    r.symbols = (syms || []).map((s) => s.name);
    const refs = await vscode.commands.executeCommand("vscode.executeReferenceProvider", uri, new vscode.Position(0, 4));
    r.references = refs.map((l) => l.range.start.line);
    const sig = await vscode.commands.executeCommand("vscode.executeSignatureHelpProvider", uri, new vscode.Position(3, 11), "[");
    r.signature = sig && sig.signatures.length ? sig.signatures[0].label : null;
    const edit = await vscode.commands.executeCommand("vscode.executeDocumentRenameProvider", uri, new vscode.Position(0, 4), "twice");
    r.rename_edits = edit ? edit.entries().reduce((n, [, es]) => n + es.length, 0) : 0;
    const actions = await vscode.commands.executeCommand("vscode.executeCodeActionProvider", uri, diags[0].range);
    r.code_actions = (actions || []).map((a) => a.title);
    // each name's kind, by the legend the server announced: `double`
    // (line 0, from character 3, six long) a function declared
    const legend = await until("the semantic tokens' legend", () => vscode.commands.executeCommand("vscode.provideDocumentSemanticTokensLegend", uri));
    const told = () => vscode.commands.executeCommand("vscode.provideDocumentSemanticTokens", uri).then((t) => (t && t.data && t.data.length ? t : null), () => null);
    const firstName = (t) => {
      const d = Array.from(t.data.slice(0, 5));
      return [d[0], d[1], d[2], legend.tokenTypes[d[3]], legend.tokenModifiers.filter((_, i) => d[4] & (1 << i)).join(" ")].join(",");
    };
    r.semantic_first = firstName(await until("semantic tokens", told));
    if (r.semantic_first !== "0,3,6,function,declaration") throw new Error("the first name told: " + r.semantic_first);
    // and shown whatever the theme says: the extension turns it on for Crow
    r.semantic_highlighting = vscode.workspace.getConfiguration("editor", { languageId: "crow" }).get("semanticHighlighting.enabled");
    if (r.semantic_highlighting !== true) throw new Error("semantic highlighting for Crow: " + r.semantic_highlighting);
    // the server killed: started again, the document told again, its
    // diagnostics back
    const first = api.server.child.pid;
    api.server.child.kill();
    await until("a new server", () => api.server.child && api.server.child.pid !== first && api.server.up);
    const again = await until("diagnostics after the restart", () => { const d = vscode.languages.getDiagnostics(uri); return d.length ? d : null; });
    r.restarted = true;
    r.diagnostics_after_restart = again.length;
    r.semantic_first_after_restart = firstName(await until("semantic tokens after the restart", told));
    r.ok = true;
  } catch (e) {
    r.error = String(e && e.stack || e);
  }
  fs.writeFileSync(OUT, JSON.stringify(r, null, 2));
  if (r.error) throw new Error(r.error);
};
