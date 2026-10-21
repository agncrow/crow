# Tools from other programs: `use mcp`

The Model Context Protocol is how a harness gets tools from another
program: a server offers tools, each a name, a description and an input
schema, and a client lists and calls them over one HTTP endpoint. `server`
names an endpoint; what it answers is a description the program can read:

```crow
use mcp

s = server['http://127.0.0.1:1/mcp']
log[s.url]
log[s.token]
t = server['https://tools.example.com/mcp', token = 'secret']
log[t.token]
bad = server['localhost:8000/mcp']
log[bad.ok]
log[bad.reason]
```

```text
http://127.0.0.1:1/mcp

secret
false
an MCP server is reached at its endpoint's URL: localhost:8000/mcp is none — http://localhost:8000/mcp
```

`discover` asks a server what it is and supports, `catalog` lists its tools
and `tool` calls one by name with its arguments, a map or a parsed value,
answering the text its result carries. A server nobody answers is the
call's error value, of kind `mcp`, and so is a result the server marks an
error:

```crow
use mcp

s = server['http://127.0.0.1:1/mcp']
d = discover[s]
log[d.ok]
log[d.kind]
ts = catalog[s]
log[ts.ok]
r = tool[s, 'weather', map[city = 'Lisbon']]
log[r.ok]
log[starts[r.reason, 'cannot reach the MCP server']]
```

```text
false
mcp
false
false
true
```

A Crow program becomes an MCP server with one route: `answer` reads the
request a POST route was handed and answers `server/discover`,
`tools/list` and `tools/call` over the functions themselves — each tool's
schema from the comment above it and the kinds its parameters take, each
call applied by name, a failure marked an error the model can read:

```crow,norun
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

`tests/mcp.rs` drives a Crow client against a stand-in server and a
stand-in client against a Crow server like this one, in both modes.
