// Crow VS Code extension: a thin shim that spawns `crow lsp` and wires it to
// the editor. All intelligence lives in the server.
const { workspace } = require("vscode");
const { LanguageClient } = require("vscode-languageclient/node");

let client;

function activate() {
  const serverPath = workspace.getConfiguration("crow").get("serverPath", "crow");
  client = new LanguageClient(
    "crow",
    "Crow Language Server",
    { command: serverPath, args: ["lsp"] },
    { documentSelector: [{ scheme: "file", language: "crow" }] }
  );
  client.start();
}

function deactivate() {
  return client ? client.stop() : undefined;
}

module.exports = { activate, deactivate };
