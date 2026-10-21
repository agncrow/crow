# mcp

Tools from other programs, and a Crow program as one: the Model Context Protocol over Streamable HTTP — a server named, its tools listed and called, and a route answering as a server over the functions themselves.


Import with `use mcp`. Tools from other programs, and a Crow program as one: the Model Context Protocol, revision 2026-07-28 — stateless, every request carrying its protocol version and the client's capabilities in `_meta`, over Streamable HTTP, one POST per message — written in Crow on `request` and the `sys` layer (design rule 20; `src/modules/mcp.co`), its functions its declaration (`server`, `discover`, `catalog`, `tool`, `answer`); `crow test src/modules/mcp.co` runs its own tests, and `tests/mcp.rs` drives a Crow client against a stand-in server and a stand-in client against a Crow server, in both modes. The design is docs/agents-design.md.

```
use mcp

s = server['http://localhost:8000/mcp']
log[catalog[s][0].name]                      --- the first tool the server offers
log[tool[s, 'weather', map[city = 'Lisbon']]] --- the text its result carries
```

| Function | Description | Example |
|----------|-------------|---|
| `server[url, token = '']` | An MCP server as a connection: the URL of its MCP endpoint (`http://` or `https://`), and the bearer token its requests carry, if any. A parsed value: `s.url`, `s.token`. An error value of kind `'mcp'` for a URL that names no endpoint | [Tools from other programs: `use mcp`](../by-example/mcp.md) |
| `discover[s]` | What the server says of itself (`server/discover`): `supportedVersions`, `capabilities`, its `instructions` for a model, and who it is under `_meta`; a parsed value | [Tools from other programs: `use mcp`](../by-example/mcp.md) |
| `catalog[s]` | The tools the server offers (`tools/list`): a parsed value list, each its `name`, `description` and `inputSchema`. A response streamed as events is read to its last message | [Tools from other programs: `use mcp`](../by-example/mcp.md) |
| `tool[s, name, given]` | A tool called with its arguments — a map, or a parsed value (`tools/call`) — answering the text its result's content carries. A result the server marks an error (`isError`) is an error value of kind `'mcp'` saying what it said, as is a tool the server does not have, a server that answers an error, and one nobody answers | [Tools from other programs: `use mcp`](../by-example/mcp.md) |
| `answer[req, tools = [f, g], name = 'crow', instructions = '']` | The request a POST route was handed, answered as an MCP server over the functions themselves: `server/discover` with the versions, the capabilities and the instructions; `tools/list` with each tool's schema from its description (`describe`, sys — the comment above the function, its parameters by name with the kinds inference found); `tools/call` with the tool of the name applied to its arguments (`apply`), its value's text the result, a wrong argument or the tool's own failure a result marked an error, a name no tool has the protocol's `Unknown tool` error. A notification is accepted with 202; a body that is no JSON, a request missing its `_meta` or the headers the transport requires (`MCP-Protocol-Version`, `Mcp-Method`, `Mcp-Name`), and one of another protocol version are refused with 400 and the JSON-RPC error the protocol names (`-32700`, `-32602`, `-32020`, `-32022`); an unknown method with 404 and `-32601` | [Tools from other programs: `use mcp`](../by-example/mcp.md) · [The standard library's layer: `use sys`](../by-example/sys.md) |

`discover`, `catalog` and `tool` reach the network through `request`, and `crow check` sees the call. A program becomes an MCP server with one route:

```
use web, mcp

--- the current weather in a city, in °C
fn weather[city]
    r = request['https://wttr.in/`trim[city]`?format=j1']
    return str[json[r.body].current_condition[0].temp_C]

fn served[req]
    return answer[req, tools = [weather], name = 'weather-server']

route['/mcp', 'POST', served]
serve[8000]
```

**Design Note: a module of its own, stateless as the protocol is.** Considered: the client inside `use llm`, since an agent is who calls a server's tools (rejected: a server needs no model, and `use llm` stays the one place a program reaches a model); the connection as a handle kind (rejected, as `reach`'s was: it holds a URL and a token's text, which a parsed value shows); the earlier revisions' `initialize` handshake and `Mcp-Session-Id` (rejected: the 2026-07-28 revision removed sessions, and a client sends one request and reads one answer, which is what `request` does; a legacy server answers 400 with no modern error, which the client names). Decided (2026-10-20): `use mcp`, the client on `request` with the headers the transport mirrors from the body, the server a function a POST route calls, the schema writer the same shape `use llm` writes for a model (a helper is a module's own, so each module writes it). The transport over stdio — a process held open as a connection — waits on a handle for a child process.



Generated by `crow test --docs --write` from the language specification, section 11 and 13. Edit the spec, not this page.
