#!/usr/bin/env bash
# The extension driven inside VS Code itself — no Node.js of ours: VS Code's
# extension test host loads test/index.js beside the extension, which asks
# every feature through VS Code's own commands against `crow lsp` and kills
# the server once to see it started again. A VS Code window opens for the
# run. Usage: editors/vscode/test.sh [path to crow] (default: the checkout's
# target/release/crow).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
crow="${1:-$here/../../target/release/crow}"
code="${CROW_VSCODE:-/Applications/Visual Studio Code.app/Contents/MacOS/Code}"
# (a path with no `//` and no link in it: the document's URI is the one the
# test asks diagnostics of — macOS's TMPDIR ends in a slash, under a link)
tmp="${TMPDIR:-/tmp}"
work="$(cd "$(mktemp -d "${tmp%/}/crow-ext.XXXXXX")" && pwd -P)"
# (VS Code's socket lives in the profile directory, whose path must be
# short: a Unix socket's is at most 103 bytes)
profile="$(mktemp -d /tmp/cx.XXXXXX)"
# (kept when the run fails, to read: VS Code's log, the server's output)
trap 'if [ -f "$work/results.json" ] && grep -q "\"ok\": true" "$work/results.json"; then rm -rf "$work" "$profile"; else echo "kept: $work $profile"; fi' EXIT
mkdir -p "$work/ws/.vscode"
cp "$here/test/ws/main.co" "$work/ws/"
printf '{ "crow.serverPath": "%s" }\n' "$(cd "$(dirname "$crow")" && pwd)/$(basename "$crow")" > "$work/ws/.vscode/settings.json"
# (run from inside VS Code — a terminal of its own, an extension's process —
# the app inherits ELECTRON_RUN_AS_NODE and VS Code's own variables, and
# starts as Node, refusing every option: they are cleared)
env -u ELECTRON_RUN_AS_NODE $(env | sed -n 's/^\(VSCODE_[A-Z_]*\)=.*/-u \1/p') \
  CROW_EXT_TEST_WS="$work/ws" CROW_EXT_TEST_OUT="$work/results.json" \
  "$code" --extensionDevelopmentPath="$here" --extensionTestsPath="$here/test/index.js" \
  --user-data-dir="$profile" --extensions-dir="$work/extensions" --disable-extensions \
  --disable-workspace-trust --skip-welcome --skip-release-notes "$work/ws" > "$work/code.log" 2>&1 || true
if [ ! -f "$work/results.json" ]; then
  echo "no results: VS Code's log follows"; tail -20 "$work/code.log"; exit 1
fi
cat "$work/results.json"
grep -q '"ok": true' "$work/results.json"
