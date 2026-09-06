# web

The server: routes and responses, auth, CORS, sessions, templates, cache, rate limits, uploads, SSE, sockets, pages, static files, mail, queues, API docs, and the HTTP client.

### Core (Server)

```
use web, ui
use views.home

serve[3000]
page['/', home.render]
```

The transpiler manages the application instance implicitly — no `app` variable needed. `page[path, view]` registers a LiveView page rendered by a view; event handlers are bound inside the view (`click = inc`), so there is no separate handler argument. `serve[port]` starts the server (`serve[port, refresh = N, budget = M]` adds periodic re-rendering and a latency budget — see Background Work). The look of the whole app is set there too: `serve[port, theme = 'dark', density = 'compact', accent = '#3b82f6', wide = true]` — `theme` is `'light'`, `'dark'`, or `'system'` (the default, following the reader's setting), `density = 'compact'` tightens the element defaults for dense screens, `accent` overrides the accent token with a hex colour, and `wide = true` lets the page use the full window instead of the reading column. A wrong value is a startup error naming the choices. The order in source doesn't matter: the transpiler always emits `serve[]` after all routes are registered.

If no routes are registered, `serve[]` shows a default welcome page so the user always sees something.

If the port is already in use, the server prints a friendly error: `error: port 3000 is already in use`.

| Function | Description | Example |
|---|---|---|
| `serve[port]` | Start the server on the given port (implicit app) | [Views: the whole element vocabulary](../by-example/views-elements.md) · [Web: responses and submodules](../by-example/web-more.md) · [Web: a JSON route](../by-example/web.md) · [View Blocks](../ch12-01-views.md) · [Reactive State and Events](../ch12-02-reactive-state.md) · [Components](../ch12-03-components.md) · [Slow Work and the Live Loop](../ch12-04-background-work.md) · [Testing a Live App](../ch12-05-testing-live-apps.md) · [Charts and Styling](../ch12-06-charts-and-style.md) · [The Element Vocabulary](../ch12-07-vocabulary.md) · [Views and Live Updates](../ch13-03-views.md) |
| `page[path, view]` | Add a LiveView page rendered by `view` | [Views: the whole element vocabulary](../by-example/views-elements.md) · [View Blocks](../ch12-01-views.md) · [Reactive State and Events](../ch12-02-reactive-state.md) · [Components](../ch12-03-components.md) · [Slow Work and the Live Loop](../ch12-04-background-work.md) · [Testing a Live App](../ch12-05-testing-live-apps.md) · [Charts and Styling](../ch12-06-charts-and-style.md) · [The Element Vocabulary](../ch12-07-vocabulary.md) · [Views and Live Updates](../ch13-03-views.md) |
| `route[path, handler]` | Add a route; `handler` is a named `fn handler[req]` that receives the request and returns a response (implicit app; `route[path, method, handler]` for other methods) | [Web: responses and submodules](../by-example/web-more.md) · [Web: a JSON route](../by-example/web.md) |

### Responses

| Function | Description | Example |
|---|---|---|
| `text[s]` | Plain text response | [Web: responses and submodules](../by-example/web-more.md) |
| `html[s]` | HTML response | [Web: responses and submodules](../by-example/web-more.md) · [Reactive State and Events](../ch12-02-reactive-state.md) · [Testing a Live App](../ch12-05-testing-live-apps.md) · [The Element Vocabulary](../ch12-07-vocabulary.md) |
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
| `render[templates, name, data]` | Render a template with data | [Views: the whole element vocabulary](../by-example/views-elements.md) · [View Blocks](../ch12-01-views.md) · [Reactive State and Events](../ch12-02-reactive-state.md) · [Components](../ch12-03-components.md) · [Slow Work and the Live Loop](../ch12-04-background-work.md) · [Testing a Live App](../ch12-05-testing-live-apps.md) · [Charts and Styling](../ch12-06-charts-and-style.md) · [The Element Vocabulary](../ch12-07-vocabulary.md) · [Views and Live Updates](../ch13-03-views.md) |

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
| `connect[url]` | Connect to a WebSocket server | [Databases: `use db`](../by-example/db.md) · [Working with Databases](../ch11-00-databases.md) · [Charts and Styling](../ch12-06-charts-and-style.md) · [Loading and Shaping the Data](../ch13-01-data.md) · [The Database Layer](../ch13-02-database.md) · [Views and Live Updates](../ch13-03-views.md) |
| `send[sock, msg]` | Send a message on a WebSocket |  |
| `recv[sock]` | Receive a message from a WebSocket |  |
| `close[sock]` | Close a WebSocket connection |  |

### LiveView / Pages

| Function | Description | Example |
|---|---|---|
| `page[path, view]` | Add a LiveView page (server-rendered with live updates via WebSocket) | [Views: the whole element vocabulary](../by-example/views-elements.md) · [View Blocks](../ch12-01-views.md) · [Reactive State and Events](../ch12-02-reactive-state.md) · [Components](../ch12-03-components.md) · [Slow Work and the Live Loop](../ch12-04-background-work.md) · [Testing a Live App](../ch12-05-testing-live-apps.md) · [Charts and Styling](../ch12-06-charts-and-style.md) · [The Element Vocabulary](../ch12-07-vocabulary.md) · [Views and Live Updates](../ch13-03-views.md) |

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
