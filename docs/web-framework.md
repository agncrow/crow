# Crow Web Framework — Design Document

## Vision

One web framework that does it all. Composable modules sharing a thin core, each independently useful, assembled by the user. Not a monolith you opt out of — independent pieces you opt into.

### Design Principles

1. **Composable** — each module works alone. `web.ws` without `web`. `web.db` without anything else.
2. **Performant** — transpiles to Rust. Native binary on server, WASM in browser. Rust-tier speed with scripting-language simplicity.
3. **Minimal coupling** — modules depend only on `core` types. Never on each other. Cross-module interaction happens in user code, not framework code.
4. **Data science native** — tensors, frames, and models are first-class in handlers. Train and serve in the same language.
5. **One way to do things** — one pattern for routes, one pattern for handlers, one pattern for state. No framework decision fatigue.
6. **Zero async complexity** — user writes sync-looking Crow code. Transpiler generates async Rust with tokio.

## Architecture Overview

### Crate: `crow_web`

One Rust crate with Cargo feature flags per module. `use web.X` in Crow source triggers the transpiler to enable only the `X` feature — minimal binary, minimal compile time.

```
crow_web/
  Cargo.toml          # feature flags per module
  src/
    lib.rs
    core/             # shared types only
      mod.rs
      request.rs
      response.rs
      headers.rs
    http/             # feature = "http"
    client/           # feature = "client"
    ws/               # feature = "ws"
    db/               # feature = "db"
    docs/             # feature = "docs"
    live/             # feature = "live"
    view/             # feature = "view"
    router/           # feature = "router"
    store/            # feature = "store"
    auth/             # feature = "auth"
    session/          # feature = "session"
    cors/             # feature = "cors"
    static_files/     # feature = "static"
    template/         # feature = "template"
    sse/              # feature = "sse"
    mail/             # feature = "mail"
    cache/            # feature = "cache"
    queue/            # feature = "queue"
    rate/             # feature = "rate"
    upload/           # feature = "upload"
```

### Module Dependency Rules

- Every module depends on `core` (Request, Response, Headers, Status, Cookie, Error).
- No module depends on any other module.
- `web.docs` reads type info emitted by the transpiler — it does not import `web`.
- `web.live` uses WebSocket transport but includes its own minimal WS code — it does not import `web.ws`.
- `web.view` is compiled to WASM — it shares `core` types but runs in a different target.

### Three Compilation Targets

| Target | What compiles | Where it runs |
|--------|--------------|---------------|
| **Server** | `web`, `web.client`, `web.db`, etc. | Native binary (x86/ARM) |
| **Client** | `web.view`, `web.router`, `web.store` | WASM in browser |
| **Docs** | `web.docs` | Static JSON + HTML (embedded in server binary) |

### Rust Backend Libraries

| Module | Rust Crate | Why |
|--------|-----------|-----|
| `web` (server) | **axum** (on hyper + tokio) | #1 Rust web framework. Routing, extractors, middleware, WebSocket, SSE built-in. Made by the tokio team. |
| `web.client` | **reqwest** (on hyper + tokio) | #1 Rust HTTP client. Async mode shares tokio runtime with server. Blocking mode works standalone without tokio. |
| `web.ws` | **axum** (built-in) | `axum::extract::ws` — no extra dependency. |
| `web.db` | **rusqlite** + **tokio-postgres** | Feature-gated: SQLite or PostgreSQL. |
| `web.auth` | **jsonwebtoken** | JWT creation/verification. |
| `web.template` | **tera** | Jinja2-style templates. |
| `web.mail` | **lettre** | SMTP email. |
| `web.cache` | **moka** + **redis** | In-memory (moka) or distributed (redis). |
| `web.upload` | **axum** (built-in) | `axum::extract::Multipart`. |
| `web.cors` | **tower-http** | `tower-http::cors` middleware. |
| `web.static` | **tower-http** | `tower-http::services::ServeDir`. |
| `web.rate` | Pure Rust | Token bucket algorithm, no external crate. |
| `web.sse` | **axum** (built-in) | `axum::response::Sse`. |
| `web.queue` | Pure Rust | Custom thread pool. |
| Response: JSON | **serde_json** | Already a dependency. |
| Response: MessagePack | **rmp-serde** | Drop-in serde integration, very stable. |
| Response: Arrow IPC | **arrow** (Apache) | Production-grade. Used by DataFusion, Polars. |

**Design decision:** axum and reqwest both build on hyper and share the tokio async runtime. When a Crow program uses both `web` (server) and `web.client` (client), they share one runtime — no wasted threads. When `web.client` is used alone (no server), reqwest's blocking mode runs without tokio.

### Library Evaluation

Why these libraries were chosen over alternatives:

| Library | Type | Async | Needs tokio | Stability | Performance | Notes |
|---------|------|-------|-------------|-----------|-------------|-------|
| **hyper** | Server/Client (low-level) | Yes | Yes | Extremely stable | Fastest HTTP in Rust | Foundation — axum and reqwest are built on it |
| **axum** | Server framework (on hyper) | Yes | Yes | Very stable | Same as hyper | Made by tokio team. #1 Rust web framework. Routing, extractors, middleware, WS, SSE built-in |
| **ureq** | Client only | **No** | No | Stable | Fine for simple use | ❌ Rejected — blocking-only, one thread per request, doesn't share runtime with server |
| **reqwest** | Client (on hyper) | Yes | Yes (or blocking) | Very stable | Same as hyper | ✅ Chosen — async shares tokio with server, blocking mode works standalone |
| **actix-web** | Server framework | Yes | Own runtime | Very stable | Excellent | ❌ Rejected — uses own runtime instead of tokio, can't share with reqwest |
| **warp** | Server framework | Yes | Yes | Stable (maintenance mode) | Good | ❌ Rejected — less active than axum, filter-based API is less intuitive |

### Server Capability Coverage (axum)

Every `web` module requirement mapped to axum support:

| Design Doc Requirement | axum Support | How |
|------------------------|-------------|-----|
| `web.new[]` — create app | ✅ | `Router::new()` |
| `route[app, path, handler]` — GET route | ✅ | `.route("/path", get(handler))` |
| `route[app, path, method, handler]` — any method | ✅ | `.route("/path", post(handler))` etc. |
| `serve[app, port]` — start server | ✅ | `axum::serve()` with tokio |
| `json[value]` — JSON response | ✅ | `Json(value)` |
| `html[string]` — HTML response | ✅ | `Html(string)` |
| `text[string]` — plain text | ✅ | Returns `String` |
| `respond[status, body]` — custom status | ✅ | `(StatusCode, body)` tuple |
| `redirect[url]` — redirect | ✅ | `Redirect::to(url)` |
| `msgpack[value]` — MessagePack | ✅ | Custom extractor + `rmp-serde` |
| `arrow[frame]` — Arrow IPC | ✅ | Custom extractor + `arrow` crate |
| `bytes[data]` — raw binary | ✅ | `axum::body::Bytes` |
| `stream[fn]` — streaming response | ✅ | `axum::body::Body::from_stream()` |
| Route params `/users/:id` | ✅ | `Path(id)` extractor |
| Query params `?page=1` | ✅ | `Query(params)` extractor |
| Request headers | ✅ | `HeaderMap` extractor |
| Request JSON body | ✅ | `Json<T>` extractor |
| WebSocket (`web.ws`) | ✅ | `axum::extract::ws` built-in |
| Static files (`web.static`) | ✅ | `tower-http::services::ServeDir` |
| CORS (`web.cors`) | ✅ | `tower-http::cors` |
| SSE (`web.sse`) | ✅ | `axum::response::Sse` built-in |
| File uploads (`web.upload`) | ✅ | `axum::extract::Multipart` built-in |
| Middleware (auth, rate limit) | ✅ | tower middleware stack |
| LiveView (WebSocket diffs) | ✅ | WS is the transport — diff logic is ours |

### Client Capability Coverage (reqwest) — ✅ Implemented

Every `web.client` requirement mapped to reqwest support:

| Design Doc Requirement | Status | Implementation |
|------------------------|--------|----------------|
| `get[url]` — GET request | ✅ Implemented | `client::get(url, None)` → `reqwest::blocking::Client::get(url).send()` |
| `get[url, headers]` — GET with headers | ✅ Implemented | `client::get(url, Some(&headers))` → `.header(k, v)` per entry |
| `post[url, body]` — POST with JSON | ✅ Implemented | `client::post(url, body, None)` → `.post(url).json(&body).send()` |
| `post[url, body, headers]` — POST with headers | ✅ Implemented | `client::post(url, body, Some(&headers))` |
| `put[url, body]` — PUT | ✅ Implemented | `client::put(url, body, None)` → `.put(url).json(&body).send()` |
| `put[url, body, headers]` — PUT with headers | ✅ Implemented | `client::put(url, body, Some(&headers))` |
| `del[url]` — DELETE | ✅ Implemented | `client::del(url, None)` → `.delete(url).send()` |
| `del[url, headers]` — DELETE with headers | ✅ Implemented | `client::del(url, Some(&headers))` |
| Response `.status` | ✅ Implemented | `CrowClientResponse.status` (u16, 0 on error) |
| Response `.body` | ✅ Implemented | `CrowClientResponse.body` (String, error message on failure) |
| Response `.json` | ✅ Implemented | `CrowClientResponse.json()` → `Option<serde_json::Value>` |
| Response `.headers` | ✅ Implemented | `CrowClientResponse.headers` (HashMap, lowercase keys) |
| Response `.ok` (2xx check) | ✅ Implemented | `CrowClientResponse.ok()` → bool |
| Works without tokio (standalone) | ✅ Implemented | `reqwest::blocking` mode, `client` feature independent of `http` |
| Works with tokio (alongside server) | ✅ Compatible | Both features can be enabled together (36 tests pass) |
| Error handling (no panics) | ✅ Implemented | Failed requests return `status: 0`, error in `.body` |

### Module Independence — What Needs Separate Crates

| Module | Covered by axum+reqwest? | Separate Crate Needed | Notes |
|--------|--------------------------|----------------------|-------|
| `web` (server) | ✅ axum | — | — |
| `web.client` | ✅ reqwest | — | — |
| `web.ws` | ✅ axum built-in | — | — |
| `web.sse` | ✅ axum built-in | — | — |
| `web.upload` | ✅ axum built-in | — | — |
| `web.cors` | ✅ tower-http | — | — |
| `web.static` | ✅ tower-http | — | — |
| `web.rate` | ✅ tower middleware | — | Pure Rust token bucket |
| `web.db` | ✅ | `rusqlite`, `tokio-postgres` | Not an HTTP concern |
| `web.docs` | ✅ | Transpiler + `serde_json` | Generation is compile-time, serving via axum |
| `web.live` | ✅ | Custom diff engine | Diff/patch logic is ours |
| `web.view` | ✅ | WASM target | Different compilation target entirely |
| `web.auth` | ✅ | `jsonwebtoken` | JWT logic, axum middleware carries it |
| `web.session` | ✅ | Pure Rust | Cookie-based, axum extracts cookies |
| `web.template` | ✅ | `tera` | Rendering engine, axum serves output |
| `web.mail` | ✅ | `lettre` | Not HTTP at all |
| `web.cache` | ✅ | `moka`, `redis` | Not HTTP at all |
| `web.queue` | ✅ | Pure Rust thread pool | Not HTTP at all |

---

## Module Catalog

### Tier 1 — Foundation

#### `core` (implicit, always available)

Shared types used by all modules. No I/O, no networking — pure data structures.

```rust
// Rust types generated by transpiler
struct CrowRequest {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    query: HashMap<String, String>,
    body: String,
    params: HashMap<String, String>,  // route params like /users/:id
}

struct CrowResponse {
    status: u16,
    headers: HashMap<String, String>,
    body: String,
}
```

Crow code accesses these naturally:

```
fn handle[req]
  log[req.method]         --- 'GET' ---
  log[req.path]           --- '/users/42' ---

```
use web

app = web.new[]

route[app, '/', fn[req]
  html['<h1>Hello World</h1>']
]

route[app, '/users', 'GET', fn[req]
  json[{users, ['alice', 'bob']}]
]

route[app, '/users', 'POST', fn[req]
  name = req.json.name
  json[{created, name}]
]

route[app, '/users/:id', 'GET', fn[req]
  json[{id, req.params.id}]
]

serve[app, env['PORT', 3000]]
```

**API:**

| Function | Description |
|----------|-------------|
| `web.new[]` | Create app instance |
| `route[app, path, handler]` | Add GET route |
| `route[app, path, method, handler]` | Add route with specific method |
| `serve[app, port]` | Start server |
| `json[value]` | Return JSON response |
| `html[string]` | Return HTML response |
| `text[string]` | Return plain text response |
| `respond[status, body]` | Return response with status code |
| `redirect[url]` | Return redirect response |
| `msgpack[value]` | Return MessagePack binary response |
| `arrow[frame]` | Return Arrow IPC response (DataFrames/tensors) |
| `bytes[data]` | Return raw binary response |
| `stream[fn]` | Return streaming/chunked response |

#### `web.client` — HTTP Client

Import: `use web.client`
Rust backend: `reqwest` (blocking mode via `reqwest::blocking`)
Feature flag: `client` (independent of `http` — no tokio required)
Rust module: `crow_web::client`

```
use web.client

--- Simple GET ---
res = get['https://api.example.com/users']
log[res.status]       --- 200 ---
log[res.ok]           --- true ---
users = res.json      --- parsed JSON ---
log[res.body]         --- raw string ---

--- POST with JSON body ---
res = post['https://api.example.com/users', {name, 'alice', age, 30}]

--- PUT ---
res = put['https://api.example.com/users/1', {name, 'bob'}]

--- DELETE ---
res = del['https://api.example.com/users/1']

--- With headers ---
res = get['https://api.example.com/data', {authorization, 'Bearer token123'}]
res = post['https://api.example.com/data', {key, 'value'}, {authorization, 'Bearer token123'}]
```

**API:**

| Function | Description |
|----------|-------------|
| `get[url]` | GET request |
| `get[url, headers]` | GET with custom headers |
| `post[url, body]` | POST with JSON body |
| `post[url, body, headers]` | POST with JSON body and custom headers |
| `put[url, body]` | PUT with JSON body |
| `put[url, body, headers]` | PUT with JSON body and custom headers |
| `del[url]` | DELETE request |
| `del[url, headers]` | DELETE with custom headers |

**Response object (`CrowClientResponse`):**

| Property | Type | Description |
|----------|------|-------------|
| `.status` | int (u16) | HTTP status code (0 if request failed) |
| `.body` | str (String) | Raw response body (error message if request failed) |
| `.json` | map/array (Option&lt;Value&gt;) | Parsed JSON, None if body is not valid JSON |
| `.headers` | map (HashMap&lt;String, String&gt;) | Response headers (lowercase keys) |
| `.ok` | bool | true if status 200-299 |

**Error handling:** If a request fails (network error, DNS failure, timeout, etc.), the response has `status: 0` and the error message in `.body`. No panics, no Result types — Crow code can simply check `res.ok` or `res.status`.

**Rust usage:**

```rust
use crow_web::client::{get, post, put, del, CrowClientResponse};
use std::collections::HashMap;

// Simple GET
let resp = get("https://api.example.com/users", None);
assert!(resp.ok());
let users = resp.json(); // Option<serde_json::Value>

// POST with JSON body
let resp = post(
    "https://api.example.com/users",
    serde_json::json!({"name": "alice", "age": 30}),
    None,
);

// GET with headers
let mut headers = HashMap::new();
headers.insert("authorization".to_string(), "Bearer token123".to_string());
let resp = get("https://api.example.com/data", Some(&headers));

// PUT
let resp = put(
    "https://api.example.com/users/1",
    serde_json::json!({"name": "bob"}),
    None,
);

// DELETE
let resp = del("https://api.example.com/users/1", None);
```

**Implementation details:**
- Uses `reqwest::blocking::Client` — works standalone without tokio
- Each function creates a new client (connection pooling handled by reqwest internally)
- JSON bodies are serialized via `serde_json::Value`
- Headers are passed as `Option<&HashMap<String, String>>`
- The `client` feature is independent of `http` (server) — they can be used together or separately

#### `web.ws` — WebSocket — ✅ Implemented

Import: `use web.ws`
Feature flag: `ws` (client-only) or `http,ws` (client + server)
Rust backend: `tungstenite` (blocking client), `axum::extract::ws` (server)
Module: `crow_web::ws`

**Client mode (standalone, no server needed):**

```
use web.ws

sock = connect['ws://localhost:8080/feed']
send[sock, 'hello']
msg = recv[sock]
log[msg]              --- 'world' ---
close[sock]
```

**Server mode (with web):**

```
use web
use web.ws

app = web.new[]

ws[app, '/chat', fn[sock, msg]
  log['received: {msg}']
  send[sock, 'echo: {msg}']
]

serve[app, env['PORT', 3000]]
```

**API:**

| Function | Rust function | Description |
|----------|--------------|-------------|
| `connect[url]` | `ws::connect(url) -> CrowWebSocket` | Connect to WebSocket server. Check `.ok()` for success. |
| `send[sock, message]` | `sock.send(msg)` | Send text message (no-op on failed socket) |
| `recv[sock]` | `sock.recv() -> Option<String>` | Receive message (blocks). Returns `None` on close/error. |
| `close[sock]` | `sock.close()` | Close connection gracefully |
| `ws[app, path, handler]` | `app.ws_route(path, handler)` | Register WebSocket handler on server (requires `http` feature) |

**Error handling:** If `connect` fails (network error, bad URL), `.ok()` returns `false` and all operations (`send`, `recv`, `close`) are safe no-ops. No panics, no `Result` unwrapping.

**Rust usage:**

```rust
// Client
use crow_web::ws;
let sock = ws::connect("ws://localhost:8080/echo");
if sock.ok() {
    sock.send("hello");
    if let Some(reply) = sock.recv() {
        println!("{reply}");
    }
    sock.close();
}

// Server (with http feature)
use crow_web::ws::{CrowWsHandler, CrowWsSender};
let app = CrowApp::new().ws_route("/echo", Arc::new(|sender: CrowWsSender, msg: String| {
    sender.send(&format!("echo: {msg}"));
}));
```

**Implementation details:**
- Client uses `tungstenite` (blocking) — works without tokio, suitable for scripts
- Server uses `axum::extract::ws` (async) — shares tokio runtime with HTTP server
- `CrowWebSocket` wraps socket in `Mutex` for thread-safe access
- `CrowWsSender` uses `mpsc::unbounded_channel` for async message passing

#### `web.sse` — Server-Sent Events — ✅ Implemented

Import: `use web.sse`
Feature flag: `sse` (enables `axum`, `tokio`, `tokio-stream`, `futures-util`)

| Function | Description |
|----------|-------------|
| `sse[app, path, handler]` | Register SSE endpoint — handler receives a sender |
| `send[sender, data]` | Push a data-only event |
| `event[sender, name, data]` | Push a named event (`event: name\ndata: data`) |

**Crow syntax:**

```
use web
use web.sse

app = web.new[]
sse[app, '/events', fn[sender]
  send[sender, 'hello']
  event[sender, 'update', 'new data']
]
serve[app, 3000]
```

**Rust API:**

```rust
use crow_web::sse::{CrowSseSender, CrowSseHandler};
use crow_web::server::app::CrowApp;
use std::sync::Arc;

let app = CrowApp::new().sse_route("/events", Arc::new(|sender: CrowSseSender| {
    sender.send("hello");
    sender.event("update", "new data");
}));
```

**Implementation details:**
- Uses `axum::response::Sse` with `tokio-stream::wrappers::ReceiverStream`
- Handler runs in `spawn_blocking` — can call `sender.send()` freely (uses `blocking_send`)
- Channel capacity: 256 events (backpressure if client is slow)
- Keep-alive enabled by default
- Feature-gated behind `sse` — independent of `http` but both can be used together

#### `web.db` — Database

Import: `use web.db`
Rust backend: `rusqlite` (SQLite) + `postgres` (PostgreSQL), feature-gated

```
use web.db

--- SQLite ---
db = open[env['DATABASE_URL', 'sqlite://app.db']]

--- PostgreSQL ---
db = open[env['DATABASE_URL', 'postgres://user:pass@localhost/mydb']]

--- Create table ---
execute[db, 'CREATE TABLE IF NOT EXISTS users (id INTEGER PRIMARY KEY, name TEXT, age INTEGER)']

--- Insert ---
execute[db, 'INSERT INTO users (name, age) VALUES (?, ?)', ['alice', 30]]

--- Query ---
rows = query[db, 'SELECT * FROM users WHERE age > ?', [25]]
for row in rows
  log['{row.name} is {row.age}']

--- Close ---
close[db]
```

**API:**

| Function | Description |
|----------|-------------|
| `open[url]` | Connect to database (error-value on failure) |
| `execute[db, sql]` | Execute statement (CREATE, INSERT, UPDATE, DELETE) |
| `execute[db, sql, params]` | Execute with parameters |
| `query[db, sql]` | Query rows → array of maps |
| `query[db, sql, params]` | Query with parameters |
| `close[db]` | Close connection |

---

### Auto-Generated

#### `web.docs` — API Documentation

Import: `use web.docs`
Generates: OpenAPI 3.0 spec + Swagger UI

```
use web
use web.docs

app = web.new[]

route[app, '/users', 'GET', fn[req]
  json[{users, ['alice', 'bob']}]
]

route[app, '/users', 'POST', fn[req]
  name = req.json.name
  age = int[req.json.age]
  json[{id, 1, name, name, age, age}]
]

docs[app, '/docs']    --- Swagger UI at /docs ---
serve[app, env['PORT', 3000]]
```

**How it works — compile-time, not runtime:**

FastAPI generates docs by inspecting Python type hints at startup. Crow does it better:

1. The transpiler already walks every `route[]` call during type inference.
2. It knows the HTTP method and path (from `route[]` args).
3. It knows what fields are accessed on `req.json` (from property access expressions in the handler body).
4. It knows the return type (from what's passed to `json[]` or `html[]`).
5. It knows error cases (from `if not x.ok` branches).
6. At compile time, it emits an OpenAPI JSON spec as a static string embedded in the binary.
7. `docs[app, '/docs']` registers a route that serves the spec + bundled Swagger UI HTML.

Zero runtime reflection. Zero Pydantic-style model definitions. The types are inferred from actual code usage.

| Comparison | FastAPI | crow_web.docs |
|------------|---------|---------------|
| Type source | Explicit Pydantic models | Inferred from handler code |
| Boilerplate | Define request/response schemas | None |
| Validation | Pydantic validates at runtime | Transpiler validates at compile time |
| Spec generation | Runtime introspection at startup | Compile-time static emission |
| Performance cost | Pydantic parsing overhead per request | Zero — types are already known |

**API:**

| Function | Description |
|----------|-------------|
| `docs[app, path]` | Serve Swagger UI at given path |

---

### Realtime

#### `web.live` — LiveView

Import: `use web.live`
Inspired by: Phoenix LiveView

Server-rendered HTML with real-time updates over WebSocket. No JavaScript framework needed on the client — just a tiny JS library (~5KB) that patches the DOM from server-sent diffs.

**How it works:**

```
Browser ←——WebSocket——→ Server
           HTML diffs
```

1. Server renders HTML for the component.
2. Server tracks component state.
3. When state changes, server re-renders and computes the HTML diff.
4. Only the diff is sent over WebSocket.
5. Client JS library patches the DOM.

**Example — Counter:**

```
use web
use web.live

app = web.new[]

live[app, '/counter', fn[state]
  state.count = state.count or 0

  h1['Count: {state.count}']
  button[click, fn[]
    state.count = state.count + 1
  , '+1']
]

serve[app, env['PORT', 3000]]
```

**Example — Real-time dashboard:**

```
use web
use web.live
use web.db

app = web.new[]
db = open[env['DATABASE_URL', 'sqlite://metrics.db']]

live[app, '/dashboard', fn[state]
  state.metrics = query[db, 'SELECT * FROM metrics ORDER BY ts DESC LIMIT 10']

  h1['Live Metrics']
  table[]
    tr[]
      th['Timestamp']
      th['Value']
    for m in state.metrics
      tr[]
        td[m.ts]
        td[m.value]
, {interval, 1000}]   --- auto-refresh every 1s ---

serve[app, env['PORT', 3000]]
```

**Comparison with Phoenix LiveView:**

| Feature | Phoenix LiveView | crow_web.live |
|---------|-----------------|---------------|
| Transport | WebSocket | WebSocket |
| Rendering | Server-side EEx templates | Server-side Crow templates |
| Diff engine | Morphdom | Custom (compile-time optimized) |
| Client JS | ~30KB | ~5KB (minimal patcher) |
| Concurrency | BEAM processes | Rust async tasks |
| Performance | Good (BEAM overhead) | Excellent (native Rust) |

**API:**

| Function | Description |
|----------|-------------|
| `live[app, path, handler]` | Register LiveView component |
| `live[app, path, handler, opts]` | With options (e.g., `{interval, 1000}` for auto-refresh) |

---

### Client-Side (WASM)

#### `web.view` — Reactive Components

Import: `use web.view`
Compiles to: WebAssembly (via `crow build --spa` or `crow build --ssr`)

Crow transpiles to Rust, and Rust has first-class `wasm32-unknown-unknown` compilation. So the pipeline is: Crow → Rust → WASM. Same language on server and client.

**Example — Todo App:**

```
use web.view
use web.store

state = store[{
  todos, [],
  input, ''
}]

fn add_todo[]
  push[state.todos, {text, state.input, done, false}]
  state.input = ''

fn toggle[i]
  state.todos[i].done = not state.todos[i].done

view[fn[]
  div[class, 'app']
    h1['Todo App']
    div[class, 'input-row']
      input[bind, state.input, placeholder, 'Add todo...']
      button[click, add_todo, 'Add']
    ul[]
      for i in range[len[state.todos]]
        todo = state.todos[i]
        li[click, fn[] toggle[i]]
          if todo.done
            s[todo.text]
          else
            span[todo.text]
]
```

**Reactivity model — fine-grained, not Virtual DOM:**

`store[]` creates a reactive state container. The transpiler tracks which parts of the view depend on which state fields. When state changes, only the affected DOM nodes update — fine-grained reactivity like Solid.js or Leptos, not Virtual DOM diffing like React.

This is possible because the transpiler sees the entire component at compile time. It knows `h1['Todo App']` is static and `span[todo.text]` depends on `state.todos`. It generates targeted DOM update code.

| Feature | Vue/React | Leptos (Rust) | crow_web.view |
|---------|-----------|---------------|---------------|
| Language | JavaScript/TypeScript | Rust | Crow |
| Compiled to | JS bundle | WASM | WASM |
| Reactivity | Virtual DOM / Proxy | Fine-grained signals | Fine-grained signals |
| Bundle size | 30-80KB (framework) | ~50KB WASM | ~30KB WASM |
| Learning curve | Medium (JSX, hooks) | High (Rust ownership, macros) | Low (Crow syntax) |
| SSR | Possible (complex) | Built-in | Built-in |

#### `web.router` — Client-Side Routing

Import: `use web.router`
Compiles to: WASM (browser-side)
Status: **Deferred** — requires `web.view` WASM target to be functional first.

Client-side URL routing. Intercepts browser navigation and swaps components without a full page reload. Equivalent to React Router, Vue Router, or Leptos Router.

```
use web.view
use web.router

routes[{
  '/', home_view,
  '/about', about_view,
  '/users/:id', user_view
}]
```

**How it works:**

1. `routes[]` registers a mapping from URL patterns to view functions.
2. On initial page load, the router matches the current URL and renders the corresponding view.
3. On link clicks, the router intercepts navigation via the History API (`pushState`), matches the new URL, and swaps the rendered component — no server round-trip.
4. Route params (`:id`) are extracted and passed to the view function as arguments.

**API:**

| Function | Description |
|----------|-------------|
| `routes[map]` | Register route table — keys are URL patterns, values are view functions |
| `navigate[path]` | Programmatic navigation (push new URL, render matching view) |
| `back[]` | Go back in browser history |

**Rust implementation plan:**

- Uses `web_sys::window().history()` for History API access.
- Listens to `popstate` events for back/forward navigation.
- Route matching uses a simple trie or linear scan with `:param` extraction.
- Integrates with `web.view` — calls the matched view function to produce a VNode tree, then diffs and patches the DOM.

**Dependencies:** `web.view` (for rendering), `wasm-bindgen`, `web-sys` (for browser APIs).

#### `web.store` — State Management

Import: `use web.store`
Compiles to: WASM (browser-side)
Status: **Deferred** — requires `web.view` WASM target to be functional first.

Reactive state container. When a value changes, all views that depend on it automatically re-render. Equivalent to Solid.js signals, Leptos signals, or Vue's `reactive()`.

```
use web.store

state = store[{
  count, 0,
  user, none
}]

--- Reactive updates ---
state.count = state.count + 1    --- triggers re-render of dependent views ---
```

**How it works:**

1. `store[]` creates a reactive state object. Each field becomes a signal.
2. The transpiler tracks which view expressions read which state fields (dependency tracking at compile time).
3. When a field is assigned, the runtime notifies only the DOM nodes that depend on that field — fine-grained reactivity, not full re-render.
4. This is possible because the transpiler sees the entire component at compile time and generates targeted update code.

**API:**

| Function | Description |
|----------|-------------|
| `store[map]` | Create reactive state container with initial values |
| `computed[fn]` | Derived value that auto-updates when dependencies change |
| `watch[fn]` | Run a side effect whenever its dependencies change |

**Example — derived state:**

```
use web.store

state = store[{items, [], filter, 'all'}]

visible = computed[fn[]
  if state.filter eq 'all'
    state.items
  else
    select[state.items, fn[item] item.status eq state.filter]
]
```

**Rust implementation plan:**

- Each signal is an `Rc<RefCell<T>>` with a subscriber list.
- `computed` creates a derived signal that re-evaluates when upstream signals change.
- `watch` registers a callback on the signal's subscriber list.
- The transpiler generates the dependency graph at compile time — no runtime tracking overhead.

**Dependencies:** `web.view` (for DOM integration), `wasm-bindgen`.

#### Design Note: `web.router` and `web.store` are deferred

These modules only make sense inside a WASM frontend. They require:

1. `web.view` compiling to `wasm32-unknown-unknown` via `wasm-bindgen`.
2. A reactive runtime that connects signals to DOM nodes.
3. Browser API access (`web-sys`) for History API, event listeners, etc.

This is equivalent to building Leptos or Yew from scratch — a significant multi-month effort. The server-side modules (`web`, `web.db`, `web.auth`, etc.) deliver value now and are the current priority. These client-side modules will be implemented when the frontend compilation pipeline is in place.

---

### Tier 2 — Essential Middleware

| Module | Import | Description | Rust Backend |
|--------|--------|-------------|-------------|
| `web.auth` | `use web.auth` | JWT creation/verification, basic auth | `jsonwebtoken` |
| `web.template` | `use web.template` | HTML templating | `tera` or custom |
| `web.session` | `use web.session` | Cookie-based sessions | Pure Rust |
| `web.static` | `use web.static` | Serve static files | `std::fs` |
| `web.cors` | `use web.cors` | CORS headers | Pure Rust |

**`web.auth` example:**

```
use web.auth

token = sign[{user_id, 42, role, 'admin'}, 'secret-key']
claims = verify[token, 'secret-key']
if not claims.ok
  log['invalid token']
```

**`web.cors` example:**

```
use web
use web.cors

app = web.new[]
cors[app, {origin, '*', methods, ['GET', 'POST']}]
```

### Tier 3 — Advanced

| Module | Import | Description | Rust Backend |
|--------|--------|-------------|-------------|
| `web.sse` | `use web.sse` | Server-sent events — ✅ Implemented | `axum::response::Sse` + `tokio-stream` |
| `web.mail` | `use web.mail` | Send email | `lettre` |
| `web.cache` | `use web.cache` | In-memory + Redis caching | `moka` + `redis` |
| `web.queue` | `use web.queue` | Background job processing | Custom thread pool |
| `web.rate` | `use web.rate` | Rate limiting | Pure Rust (token bucket) |
| `web.upload` | `use web.upload` | Multipart file uploads | `multer` |

---

## Rendering Modes

**Status: Deferred** — depends on `web.view`, `web.router`, and `web.store` WASM compilation.

The same Crow source code can be compiled with different strategies:

| Mode | Command | What happens | Dependencies |
|------|---------|-------------|--------------|
| **SPA** | `crow build --spa` | Full WASM app, client-side routing | `web.view`, `web.router`, `web.store` |
| **SSR** | `crow build --ssr` | Server renders HTML, WASM hydrates on client | `web.view`, `web.router`, `web` |
| **LiveView** | `crow build --live` | Server renders + WebSocket diffs, minimal client JS | `web.live` (✅ implemented) |
| **Static** | `crow build --static` | Pre-renders all pages to HTML at build time | `web.view`, `web.template` |

The user doesn't change code — just the build flag.

**Current state:** `--live` is the closest to working — `web.live` already has the LCS diff engine and WebSocket push. The other three modes require the WASM frontend pipeline (`wasm-bindgen`, `wasm32-unknown-unknown` target, browser DOM access) which is not yet implemented. See the Design Note under `web.router` and `web.store` for details.

---

## Full-Stack Compilation Pipeline

```
app.crow (single source)
    │
    ├── crow build ──→ Transpiler splits server/client code
    │                     │
    │                     ├── server.rs ──→ cargo build ──→ Native binary (HTTP server)
    │                     │
    │                     └── client.rs ──→ cargo build --target wasm32 ──→ app.wasm (browser)
    │
    └── The native binary serves app.wasm + handles API routes
```

---

## Configuration

Crow uses environment variables as the single configuration mechanism. No YAML, no TOML, no custom config format — one way to do things.

### `env` builtin

| Form | Behavior |
|------|----------|
| `env['PORT']` | Read env var. Returns error-value if not set. |
| `env['PORT', 3000]` | Read env var. Returns `3000` if not set. |

### `.env` auto-loading

`crow run` and `crow build` automatically load a `.env` file from the project root if it exists. Real environment variables take priority over `.env` values.

```
# .env
PORT=3000
DATABASE_URL=postgres://user:pass@localhost/mydb
SECRET_KEY=abc123
```

```
--- app.crow ---
use web
use web.db

db = open[env['DATABASE_URL']]
app = web.new[]

route[app, '/users', fn[req]
  json[query[db, 'SELECT * FROM users']]
]

serve[app, env['PORT', 3000]]
```

No imports needed. No dotenv library. The runtime loads `.env` automatically.

---

## Data Science Integration

The killer differentiator: train a model and serve it in the same language, compiled to native Rust performance, with zero framework boilerplate.

```
use math
use math.nn
use web

model = load['model.crow']

app = web.new[]
route[app, '/predict', 'POST', fn[req]
  input = tensor[req.json.data]
  output = model[input]
  json[{prediction, output}]
]
serve[app, env['PORT', 3000]]
```

No GIL. No serialization boundary. No Python-to-Go rewrite. Same language, compiled to Rust speed.

### Response Formats for ML/Data Science

JSON is universal but slow for large data. Crow supports multiple wire formats optimized for different use cases:

| Format | Function | Use Case | Why |
|--------|----------|----------|-----|
| **JSON** | `json[value]` | API responses, small data | Universal, human-readable. Slow for large tensors. |
| **MessagePack** | `msgpack[value]` | API responses, medium data | Binary JSON — 2-5x smaller, 10x faster to parse than JSON. |
| **Arrow IPC** | `arrow[frame]` | DataFrame/tensor transfer | Zero-copy columnar format — no serialization overhead. |
| **Raw bytes** | `bytes[data]` | Model weights, binary blobs | Zero format overhead — just send the bytes. |
| **Streaming** | `stream[fn]` | Batch inference, large results | Stream results as computed, don't buffer in memory. |

#### Performance comparison

| Scenario | JSON | MessagePack | Arrow IPC |
|----------|------|-------------|-----------|
| 10KB API response | ~0.1ms | ~0.02ms | overkill |
| 1MB tensor | ~50ms | ~10ms | ~1ms |
| 10MB DataFrame | ~500ms | ~100ms | ~5ms |
| 100MB dataset | ~5s | ~1s | ~50ms |

Arrow IPC is the standout for data science: Python (pandas/polars), R, Julia, and Spark can all read it with **zero deserialization** — just memory-map the bytes.

#### Rust crate backends

| Format | Rust Crate | Maturity | Notes |
|--------|-----------|----------|-------|
| JSON | `serde_json` | Very stable | Already a dependency |
| MessagePack | `rmp-serde` | Very stable | Drop-in serde integration |
| Arrow IPC | `arrow` (Apache) | Very stable, production-grade | Used by DataFusion, Polars |
| Protobuf | `prost` | Very stable | Schema-defined, smallest wire size |
| CBOR | `ciborium` | Stable | Self-describing binary, good for IoT/embedded ML |
| Streaming | Built into axum | Native | Chunked transfer encoding |
| Raw bytes | No crate needed | — | Just `Vec<u8>` |

#### Example — same model, multiple formats

```
use web
use math.nn

model = load['model.crow']
app = web.new[]

--- JSON (small, human-readable) ---
route[app, '/predict', 'POST', fn[req]
  result = model[tensor[req.json.data]]
  json[{prediction, result}]
]

--- MessagePack (fast, binary — 10x faster than JSON) ---
route[app, '/predict/fast', 'POST', fn[req]
  result = model[tensor[req.json.data]]
  msgpack[{prediction, result}]
]

--- Arrow IPC (zero-copy DataFrame response) ---
route[app, '/data', fn[req]
  df = query[db, 'SELECT * FROM metrics']
  arrow[df]
]

--- Streaming batch inference ---
route[app, '/batch', 'POST', fn[req]
  stream[fn[send]
    for batch in chunks[req.json.data, 100]
      pred = model[tensor[batch]]
      send[pred]
  ]
]

--- Raw model weights download ---
route[app, '/model/weights', fn[req]
  bytes[read['model.bin']]
]

serve[app, env['PORT', 3000]]
```

The client picks the format based on the endpoint or `Accept` header. Same model, different wire formats for different performance needs.

#### When to use which format

| You need... | Use |
|-------------|-----|
| Browser-friendly API | `json` |
| Fast API for Python/Go/Rust clients | `msgpack` |
| Transfer DataFrames to pandas/polars | `arrow` |
| Download model weights or files | `bytes` |
| Predict on millions of rows | `stream` |
| Real-time training progress | SSE (`web.sse`) |

#### Design Note: No Protobuf or CBOR builtins

Protobuf requires schema files (`.proto`) and code generation — this violates Crow's "zero boilerplate" principle. CBOR is niche. Both can be supported via packages (`crow install protobuf`, `crow install cbor`) if needed, but they are not core response types. MessagePack covers the "fast binary" use case without schemas, and Arrow IPC covers the "zero-copy data science" use case. Two formats, two use cases, no overlap.

---

## Implementation Phases

| Phase | Modules | Effort | What it unlocks |
|-------|---------|--------|----------------|
| **Phase 1** | core, web, web.client | 2 weeks | API servers, HTTP requests |
| **Phase 2** | web.db, web.docs | 2 weeks | Data apps, auto documentation |
| **Phase 3** | web.ws ✅, web.live | 2 weeks | Real-time apps without JS |
| **Phase 4** | web.view, web.router, web.store | 4 weeks | Full frontend in Crow (WASM) |
| **Phase 5** | web.auth, web.session, web.cors, web.static | 2 weeks | Production-ready middleware |
| **Phase 6** | web.sse, web.mail, web.cache, web.queue, web.rate, web.upload | 3 weeks | Enterprise features |

### Transpiler Codegen Status

All server-side web modules have transpiler codegen (Crow → Rust), validator rules, and interpreter mock support:

| Module | Crow Syntax | Transpiler | Interpreter |
|--------|------------|------------|-------------|
| `web` (core) | `web.new[]`, `route[]`, `serve[]` | ✅ | ✅ |
| `web.client` | `get[]`, `post[]`, `put[]`, `del[]` | ✅ | ✅ |
| `web.db` | `sqlite[]`, `exec[]`, `query[]` | ✅ | ✅ |
| `web.auth` | `sign[]`, `verify[]` | ✅ | ✅ |
| `web.cors` | `cors[app]`, `cors[app, origins]` | ✅ | ✅ |
| `web.session` | `sessions[]` | ✅ | ✅ |
| `web.template` | `templates[glob]`, `render[t, name, data]` | ✅ | ✅ |
| `web.cache` | `cache[max]`, `cache[max, ttl]` | ✅ | ✅ |
| `web.rate` | `limit[max, window]` | ✅ | ✅ |
| `web.upload` | `uploads[req]` | ✅ | ✅ |
| `web.sse` | `sse[app, path, handler]` | ✅ | ✅ |
| `web.ws` | `ws[app, path, handler]`, `connect[]`, `send[]`, `recv[]`, `close[]` | ✅ | ✅ |
| `web.live` | `live[app, path, handler]` | ✅ | ✅ |
| `web.static` | `static_dir[app, url_path, dir_path]` | ✅ | ✅ |
| `web.mail` | `smtp[host, port, user, pass]` | ✅ | ✅ |
| `web.queue` | `web.queue[n]`, `push[queue, fn]` | ✅ | ✅ |
| `web.docs` | `web.docs[title, version]` | ✅ | ✅ |

**Deferred:** `web.view`, `web.router`, `web.store` (WASM client-side compilation).

---

## Competitive Position

| Capability | Rails | Django | Phoenix | FastAPI | Next.js | Leptos | **crow_web** |
|-----------|-------|--------|---------|---------|---------|--------|------------|
| Server HTTP | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| HTTP client | ❌ | ❌ | ❌ | ❌ | ✅ | ✅ | ✅ |
| WebSocket | ✅ | ❌ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Database | ✅ | ✅ | ✅ | ❌ | ❌ | ❌ | ✅ |
| Auto API docs | ❌ | ❌ | ❌ | ✅ | ❌ | ❌ | ✅ |
| LiveView | ❌ | ❌ | ✅ | ❌ | ❌ | ❌ | ✅ |
| WASM frontend | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | ✅ |
| Reactive components | ❌ | ❌ | ✅ | ❌ | ✅ | ✅ | ✅ |
| SSR + hydration | ❌ | ❌ | ✅ | ❌ | ✅ | ✅ | ✅ |
| Data science native | ❌ | ❌ | ❌ | ⚠️ | ❌ | ❌ | ✅ |
| ML response formats (msgpack, arrow) | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ |
| Streaming inference | ❌ | ❌ | ❌ | ⚠️ | ❌ | ❌ | ✅ |
| Compiled performance | ❌ | ❌ | ⚠️ | ❌ | ❌ | ✅ | ✅ |
| True modularity | ❌ | ❌ | ❌ | ⚠️ | ⚠️ | ✅ | ✅ |
