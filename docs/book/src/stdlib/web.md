# web

The server: routes and responses, auth, CORS, sessions, templates, cache, rate limits, uploads, SSE, sockets, pages, static files, mail, queues, API docs, and the HTTP client.

### Core (Server)

```
use web, ui
use views.home

serve[3000]
page['/', home.render]
```

The transpiler manages the application instance implicitly — no `app` variable needed. `page[path, view]` registers a LiveView page rendered by a view; event handlers are bound inside the view (`click = inc`), so there is no separate handler argument. `serve[port]` starts the server (`serve[port, refresh = N, budget = M]` adds periodic re-rendering and a latency budget — see Background Work). `page[path, view, url = ['day', 'sym']]` puts those facts in the page's address: a session starts from the values in the query string (`/?day=2026-09-04&sym=IMRN`), each read in its fact's shape, and the address follows them as they change (`history.replaceState`), so a reload or a shared link opens the same view — a fact of a part is named by its path, `url = ['session.day']`, and appears by its last name. The look of the whole app is set there too: `serve[port, theme = 'dark', density = 'compact', accent = '#3b82f6', wide = true]` — `theme` is `'light'`, `'dark'`, or `'system'` (the default, following the reader's setting), `density = 'compact'` tightens the element defaults for dense screens, `accent` overrides the accent token with a hex colour, and `wide = true` lets the page use the full window instead of the reading column. A wrong value is a startup error naming the choices. The order in source doesn't matter: the transpiler always emits `serve[]` after all routes are registered.

If no routes are registered, `serve[]` shows a default welcome page so the user always sees something.

If the port is already in use, the server prints a friendly error: `error: port 3000 is already in use`.

| Function | Description | Example |
|---|---|---|
| `serve[port]` | Start the server on the given port (implicit app) | [Views: the whole element vocabulary](../by-example/views-elements.md) · [Web: responses and submodules](../by-example/web-more.md) · [Web: a JSON route](../by-example/web.md) · [View Blocks](../ch12-01-views.md) · [Reactive State and Events](../ch12-02-reactive-state.md) · [Components](../ch12-03-components.md) · [Slow Work and the Live Loop](../ch12-04-background-work.md) · [Testing a Live App](../ch12-05-testing-live-apps.md) · [Charts and Styling](../ch12-06-charts-and-style.md) · [The Element Vocabulary](../ch12-07-vocabulary.md) · [Derivations](../ch12-08-derive.md) · [Views and Live Updates](../ch13-03-views.md) |
| `page[path, view]` | Add a LiveView page rendered by `view` | [Views: the whole element vocabulary](../by-example/views-elements.md) · [View Blocks](../ch12-01-views.md) · [Reactive State and Events](../ch12-02-reactive-state.md) · [Components](../ch12-03-components.md) · [Slow Work and the Live Loop](../ch12-04-background-work.md) · [Testing a Live App](../ch12-05-testing-live-apps.md) · [Charts and Styling](../ch12-06-charts-and-style.md) · [The Element Vocabulary](../ch12-07-vocabulary.md) · [Derivations](../ch12-08-derive.md) · [Views and Live Updates](../ch13-03-views.md) |
| `route[path, handler]` | Add a route; `handler` is a named `fn handler[req]` that receives the request and returns a response (implicit app; `route[path, method, handler]` for other methods) | [Web: responses and submodules](../by-example/web-more.md) · [Web: a JSON route](../by-example/web.md) |

### Responses

| Function | Description | Example |
|---|---|---|
| `text[s]` | Plain text response | [Web: responses and submodules](../by-example/web-more.md) |
| `html[s]` | HTML response | [Web: responses and submodules](../by-example/web-more.md) · [Reactive State and Events](../ch12-02-reactive-state.md) · [Testing a Live App](../ch12-05-testing-live-apps.md) · [The Element Vocabulary](../ch12-07-vocabulary.md) · [Derivations](../ch12-08-derive.md) |
| `json[map]` | JSON response from a map | [JSON](../by-example/json.md) · [Web: a JSON route](../by-example/web.md) |
| `respond[status, body]` | Response with custom status code | [Web: responses and submodules](../by-example/web-more.md) |
| `redirect[url]` | HTTP redirect response | [Web: responses and submodules](../by-example/web-more.md) |
| `msgpack[value]` | MessagePack binary response (requires `rmp-serde`) | [Web: responses and submodules](../by-example/web-more.md) |
| `arrow[frame]` | Arrow IPC binary response for DataFrames | [Web: responses and submodules](../by-example/web-more.md) |
| `bytes[data]` | Raw binary response (`application/octet-stream`) | [Web: responses and submodules](../by-example/web-more.md) |
| `stream[handler_fn]` | Streaming/chunked response via channel |  |

### Auth (`use web.auth`)

| Function | Description | Example |
|---|---|---|
| `sign[claims, secret]` | Sign a JWT from a claims map | [Math: tensors](../by-example/math.md) |
| `verify[token, secret]` | Verify a JWT and return claims map |  |

### CORS (`use web.cors`)

| Function | Description | Example |
|---|---|---|
| `cors[app]` | Apply permissive CORS (allow all origins) |  |
| `cors[app, origins]` | Apply CORS with specific allowed origins |  |

### Sessions (`use web.session`)

| Function | Description | Example |
|---|---|---|
| `sessions[]` | Create an in-memory session store | [Web: responses and submodules](../by-example/web-more.md) · [The Element Vocabulary](../ch12-07-vocabulary.md) |

### Templates (`use web.template`)

| Function | Description | Example |
|---|---|---|
| `templates[glob]` | Load templates matching a glob pattern |  |
| `render[templates, name, data]` | Render a template with data | [Views: the whole element vocabulary](../by-example/views-elements.md) · [View Blocks](../ch12-01-views.md) · [Reactive State and Events](../ch12-02-reactive-state.md) · [Components](../ch12-03-components.md) · [Slow Work and the Live Loop](../ch12-04-background-work.md) · [Testing a Live App](../ch12-05-testing-live-apps.md) · [Charts and Styling](../ch12-06-charts-and-style.md) · [The Element Vocabulary](../ch12-07-vocabulary.md) · [Derivations](../ch12-08-derive.md) · [Views and Live Updates](../ch13-03-views.md) |

### Cache (`use web.cache`)

| Function | Description | Example |
|---|---|---|
| `cache[max]` | Create an LRU cache with max entries | [Web: responses and submodules](../by-example/web-more.md) |
| `cache[max, ttl]` | Create an LRU cache with max entries and TTL in seconds | [Web: responses and submodules](../by-example/web-more.md) |

### Rate Limiting (`use web.rate`)

| Function | Description | Example |
|---|---|---|
| `limit[max, window]` | Create a rate limiter (max requests per window seconds) | [Web: responses and submodules](../by-example/web-more.md) |

### File Uploads (`use web.upload`)

| Function | Description | Example |
|---|---|---|
| `uploads[req]` | Extract uploaded files from a request |  |

### SSE (`use web.sse`)

| Function | Description | Example |
|---|---|---|
| `sse[app, path, handler]` | Add a Server-Sent Events endpoint |  |

### WebSocket (`use web.ws`)

| Function | Description | Example |
|---|---|---|
| `ws[app, path, handler]` | Add a WebSocket endpoint | [Structured files: `use io`](../by-example/io.md) · [Reactive State and Events](../ch12-02-reactive-state.md) |
| `connect[url]` | Connect to a WebSocket server | [Databases: `use db`](../by-example/db.md) · [Working with Databases](../ch11-00-databases.md) · [Reactive State and Events](../ch12-02-reactive-state.md) · [Slow Work and the Live Loop](../ch12-04-background-work.md) · [Testing a Live App](../ch12-05-testing-live-apps.md) · [Charts and Styling](../ch12-06-charts-and-style.md) · [Derivations](../ch12-08-derive.md) · [Loading and Shaping the Data](../ch13-01-data.md) · [The Database Layer](../ch13-02-database.md) · [Views and Live Updates](../ch13-03-views.md) |
| `send[sock, msg]` | Send a message on a WebSocket |  |
| `recv[sock]` | Receive a message from a WebSocket |  |
| `close[sock]` | Close a WebSocket connection |  |

### LiveView / Pages

| Function | Description | Example |
|---|---|---|
| `page[path, view]` | Add a LiveView page (server-rendered with live updates via WebSocket) | [Views: the whole element vocabulary](../by-example/views-elements.md) · [View Blocks](../ch12-01-views.md) · [Reactive State and Events](../ch12-02-reactive-state.md) · [Components](../ch12-03-components.md) · [Slow Work and the Live Loop](../ch12-04-background-work.md) · [Testing a Live App](../ch12-05-testing-live-apps.md) · [Charts and Styling](../ch12-06-charts-and-style.md) · [The Element Vocabulary](../ch12-07-vocabulary.md) · [Derivations](../ch12-08-derive.md) · [Views and Live Updates](../ch13-03-views.md) |
| `download[value, name]` | Hand the reader's browser a file when the handler commits: a frame as CSV, a string as text, a picked file's bytes | [Reactive State and Events](../ch12-02-reactive-state.md) |

**A page and its session.** The page request renders the reader's session once, and the page's socket adopts that session — its facts, its derivations and the render already on screen — so a page loads with one render. **A session outlives its connection.** When the socket closes — a laptop sleeps, the network drops, a rebuild restarts nothing but the page — the server keeps the session for ten minutes, and the page, which holds the session's token for its tab, resumes it on reconnect: the same facts, a replay still where it was, sent its render at once. A waiting session keeps its facts and its derivations' last values, so it comes back without a flash of loading; at most a thousand sessions wait at once. The token is a credential and is treated as one: it travels only over the socket, it is good once, and every resume replaces it, so a copied token resumes nothing after its tab has. Reloading the tab resumes it too; a second tab is a session of its own. A socket that brings no live token — a session past its ten minutes, a server that restarted — starts a fresh session and is sent its render at once: the page never keeps showing a session the server no longer holds. `serve[…, keep = true]` keeps sessions across a restart as well: each session's facts are written to `.crow/sessions/` as they change, and a tab that comes back to a restarted program finds its session there for the same ten minutes; a fact whose name or type changed since starts from its declared value, which the log says. The directory ignores itself in version control. While the socket is down the page dims, says *reconnecting…* and takes no input, so a click is never silently lost. Events that arrive while a render runs are applied together and answered by one render: a slider dragged across its range is one render, not one per step. A render sends what changed, not the page: the server reads the render as the browser builds it — a chart's marks and a table's rows included — compares it with the render the page holds, matching elements by `id` and then by position as the page does, and sends the changes (a text, an attribute, an element added, gone or replaced) when they are smaller than the page, the whole page when they are not. A moving play-head on a chart is a few attributes, not the drawing again, and a row added to a list whose items carry ids is one row. Each change names the element it expects to find; a page that meets one it cannot place — its shape changed under it — asks for the whole page back, and a field being typed in keeps the reader's draft. Markup the browser would have to repair (a block inside a paragraph, a row outside a table, a tag closed by its parent's end tag) is sent as whole pages, and the server log says which repair, once. Handlers and renders run on worker threads, never on the threads that carry the sockets, so a session's slow work delays only its own page. A clicked element is marked `data-crow-busy` the moment it is clicked, before the server answers, and the mark clears when the answer to that click arrives — the reader sees the click taken in the same frame, with no program code in the browser.

**A page sends only what it was offered.** A session accepts a click only when a render it sent carried that handler with exactly those arguments (`click = set_speed[10.0]` offers `set_speed` with `10.0`, a table row its key), a bound input or select only when a render drew it, and a form only when a render drew it. A helper no element names, a fact no input binds, and the button of a closed modal or a hidden element are refused — a confirmation's second step is accepted only after its first opened it. A refusal changes nothing and is logged: `crow live: refused 'name' — the page did not offer it`. The socket opens only from the app's own page (a page on another site is refused before it can send anything), and a served app listens on this machine only; set `CROW_HOST=0.0.0.0` where the network must reach it — a container, a server behind a proxy.

Both modes serve pages. `crow run file.co` compiles and serves; `crow run -i file.co` serves from the interpreter at once, with no Rust compile between an edit and the page — the live runtime keeps the sessions (their facts, the commit, the notice, the questions, the address, the reconnects) in both, and only who renders the views and runs the handlers differs. One difference is of timing, not of meaning: the interpreter computes a derivation where it is read, so a page served by it waits for its queries, where the compiled page shows the last value and catches up. `crow test --served` drives every page in `tests/served/`, and every app under `examples/` with a `main.events` script beside its `main.co` (served from a fresh copy, so its database starts empty in each mode), with the same events in both modes and compares the pages they settle on.

### Static Files (`use web.static`)

| Function | Description | Example |
|---|---|---|
| `static_dir[app, url_path, dir_path]` | Serve static files from a directory |  |

### Mail (`use web.mail`)

| Function | Description | Example |
|---|---|---|
| `smtp[host, port, user, pass]` | Create an SMTP mailer | [Web: responses and submodules](../by-example/web-more.md) |

### Queue (`use web.queue`)

| Function | Description | Example |
|---|---|---|
| `web.queue[n]` | Create a background task queue with n workers |  |
| `push[queue, fn]` | Push a task onto the queue | [Arrays](../by-example/arrays.md) · [Arrays and Tensors](../ch05-01-arrays-and-tensors.md) |

### API Docs (`use web.docs`)

| Function | Description | Example |
|---|---|---|
| `web.docs[title, version]` | Generate OpenAPI docs with Swagger UI |  |

### Embedded SQLite (`use web.db`, legacy)

| Function | Description | Example |
|---|---|---|
| `sqlite[path]` | Open an embedded SQLite handle for `query`/`exec` inside a web app. Superseded by the `db` module (`connect['sqlite://path']`), which works everywhere and returns error values; prefer that. |  |

### Client (`use web.client`)

An outbound HTTP client (`get`, `post`, …) is on the roadmap and not yet part of the language. Today a page pulls remote data with `fetch[url]` / `poll[url, seconds]` (reactive, see above), and scripts shell out (`shell['curl …']`).



Generated by `crow test --docs --write` from the language specification, section 11 and 13. Edit the spec, not this page.
