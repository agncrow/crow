# web and ui

Views, reactive state, the layout vocabulary, charts, styling, and the server: routes, responses, auth, sessions, sockets, and more.


Import sub-modules with `use web.X`. Each sub-module enables only its feature — minimal binary, minimal compile time. See `docs/web-framework.md` for full architecture details.

### View Blocks — the UI Vocabulary Zone

UI trees are built in `view` blocks — a declaration form, sibling of `fn`:

```
use web, ui

count = 0

fn inc[]
    count = count + 1

view render[]
    return div[
        h1['Count: `count`'],
        button['+', click = inc]
    ]

serve[3000]
page['/', render]
```

**Inside a view, the standard HTML element vocabulary is active** — `div`, `span`, `p`, `h1`, `h2`, `h3`, `h4`, `h5`, `h6`, `button`, `ul`, `ol`, `li`, `form`, `a`, `nav`, `header`, `footer`, `section`, `tr`, `td`, `th`, `thead`, `tbody`, `option`, `textarea`, `label`, `pre`, `code`, `input`, `img`, `table` — with their exact HTML names. **Outside a view, those names don't exist**: `p = 0.95`, `fn add[a, b]`, and `form = req.body` are ordinary code in any ui program, and `use ui` reserves nothing in your file. Calling an element outside a view is an error that points at the fix.

Views compose: a view calls other views (components) and ordinary functions freely. A module can export views — importers reach them qualified, like any module name:

```
--- cards.crow
view card[title, body_text]
    div[h2[title], p[body_text]]

--- main.crow
use cards as c

view render[]
    div[h1['Home'], c.card['Status', 'All good']]
```

This is the JSX/SwiftUI boundary: the lowercase vocabulary is fixed and language-owned; your components are module-exported views. There is no way to add elements or define new vocabulary zones — see the Design Note "Names Are Never Ambiguous".

Views and event handlers are the only functions that see reactive state (see Function Scope). `crow fmt --fix-views` migrates pre-view code (`fn` declarations that build ui trees) automatically.

**The view tree is target-independent.** A view describes a tree; what renders it is a build target. Web today; a desktop or mobile shell renders the same tree without a language change. Platforms are build targets, never dialects — there will never be a `screen` or `window` vocabulary fork.

### Reactive Data and Inputs (`use ui`)

Top-level variables in a ui program are reactive state (see Function Scope). Three builtins declare state that the runtime keeps current, and bind inputs to it:

| Function | Description | Example |
|---|---|---|
| `fetch[url]` | Reactive state loaded once by HTTP GET when the page connects | [Views: the whole element vocabulary](../by-example/views-elements.md) |
| `poll[url, seconds]` | Reactive state re-fetched every `seconds`; every reader re-renders when it changes | [Views: the whole element vocabulary](../by-example/views-elements.md) |
| `field[label, var]` | A labeled input two-way bound to reactive variable `var` (inside a view) | [Views: the whole element vocabulary](../by-example/views-elements.md) · [Testing a Live App](../ch12-05-testing-live-apps.md) · [Charts and Styling](../ch12-06-charts-and-style.md) · [Views and Live Updates](../ch13-03-views.md) |

### Views render wherever they are called

A `view` returns its markup from any call site — a `page[]`, the test harness, another view, or a program writing pages to disk (`doc.document[title, chapter[...]]`). Inside an element, a string child that starts and ends with a tag passes through as markup; any other string is text and is escaped. Both renderers apply the same rule.

## Layout — the vocabulary

Pages are composed from layout primitives, never styled with CSS. Five containers join the element vocabulary inside views, and a fixed set of layout attributes applies to any element. Each has one written rule; the kernel table (`crow_runtime::layout`) turns it into the CSS a browser lays out with, and the layout engine — written in Crow, shipped inside the compiler — implements the same rule for tests and the desktop shell. Design and rationale: `ui-engine-design.md`.

| Container | Rule | Example |
|---|---|---|
| `stack[a, b, ...]` | children top to bottom, each the container's full content width | [Frames: `use frame`](../by-example/frames.md) · [Math: shape operations](../by-example/math-shape.md) · [Views: the whole element vocabulary](../by-example/views-elements.md) · [View Blocks](../ch12-01-views.md) · [Charts and Styling](../ch12-06-charts-and-style.md) |
| `row[a, b, ...]` | children left to right on one line; overflow is a failure `fits[]` reports, never a silent wrap (`wrap = true` opts in) | [Views: the whole element vocabulary](../by-example/views-elements.md) · [Web: responses and submodules](../by-example/web-more.md) · [Charts and Styling](../ch12-06-charts-and-style.md) |
| `grid[a, b, ..., columns = 2]` | cells left-to-right, top-to-bottom, equal columns; `columns = '240px fill'` names tracks | [Views: the whole element vocabulary](../by-example/views-elements.md) · [Charts and Styling](../ch12-06-charts-and-style.md) |
| `overlay[base, top]` | `top` positioned over `base` by `anchor = 'center' \| 'top_right' \| ...` | [Views: the whole element vocabulary](../by-example/views-elements.md) · [Charts and Styling](../ch12-06-charts-and-style.md) |
| `scroll[child, height = '360px']` | a fixed-height viewport; the platform scrolls | [Views: the whole element vocabulary](../by-example/views-elements.md) · [Charts and Styling](../ch12-06-charts-and-style.md) |

| Attribute | Values | Example |
|---|---|---|
| `gap`, `pad` (`'m'` or `'m l'`), `margin` | the spacing scale `xs s m l xl` = 4 8 16 24 32 px |  |
| `align` | `start center end stretch` (cross axis) |  |
| `justify` | `start center end between around` (main axis) |  |
| `width`, `height`, `min_width`, `max_width` | `'240px'`, `'50%'` (width only), `'fill'` for remaining space; `aspect = '16/9'` |  |
| `size`, `weight` | the type scale `xs s m l xl xxl` = 12 14 16 20 24 32 px; `regular medium bold` |  |
| `surface = 'card'`, `radius`, `border = true`, `shadow` | surfaces: radius `none s m l round`, shadow `s m l` |  |
| `color`, `background` | a colour token: `text muted accent up down surface bg` — never a literal colour |  |
| `lines = 2` | clamp a wrapping paragraph | [Structured files: `use io`](../by-example/io.md) |
| `below = 'phone' \| 'tablet'`, `becomes = 'stack'`, `columns_then = 1`, `hidden_below` | responsive rules at named widths (phone < 600, tablet < 1024) — no media queries |  |

Layout attributes are literals; a wrong value is a compile error naming the scale (`gap = 'huge' is not a spacing step (xs s m l xl)`), the same in both modes. Attributes outside the vocabulary (`class`, `href`, `name`, ...) pass through as HTML. Labels, headings, buttons, and cells are single-line; only `p` wraps.

### Charts — `chart[frame, ...]`

A chart is a view over a frame. `chart` is an element like `table`: valid only inside views, it takes a frame plus named arguments that say which columns to draw, and renders server-side as SVG:

```
view render[]
    bars = query[conn, 'SELECT ts, o, hi, lo, c, vol FROM bars ORDER BY ts']
    return div[
        chart[bars, candles = ['o', 'hi', 'lo', 'c'], bars_y = 'vol',
              marker = open_bell, tz = 'America/New_York', height = 200]
    ]
```

| Named argument | Default | Meaning | Example |
|---|---|---|---|
| `x` | `'ts'` | Column of moments for the x axis | [Math: linear algebra](../by-example/math-linalg.md) · [Math: reductions, sorting, and selection](../by-example/math-reductions.md) · [Math: special functions and complex numbers](../by-example/math-special.md) · [Math: tensors and linear algebra](../by-example/math.md) · [Neural-network building blocks: `use math.nn`](../by-example/nn.md) · [Numbers and conversion](../by-example/numbers.md) · [Views: the whole element vocabulary](../by-example/views-elements.md) · [Testing](../ch09-00-testing.md) · [Charts and Styling](../ch12-06-charts-and-style.md) |
| `candles` | `[]` | Four column names — open, high, low, close — drawn as candlesticks |  |
| `lines` | `[]` | Column names drawn as line series | [Structured files: `use io`](../by-example/io.md) |
| `bars_y` | `''` | Column drawn as a volume pane under the chart |  |
| `marker` | `0` | A moment marked with a vertical play-head |  |
| `tz` | `'UTC'` | Timezone for the axis ticks | [Frames: `use frame`](../by-example/frames.md) |
| `height` | `320` | Height in pixels |  |

Colors come from the style tokens (`--crow-up`, `--crow-down`, `--crow-accent`), so every chart is theme-correct with no color plumbing. An empty frame renders a placeholder rather than an error; a frame over the point budget (1000 points) refuses with an error that says to aggregate first — coarser rows from the query, or `head[frame, n]` — never a silent downsample. The geometry and the SVG assembly live in `crow_runtime::chart`, so both modes emit identical markup. Design and roadmap: `chart-module-design.md`.

### Styling — the default stylesheet

Pages look right with zero CSS: every page shell carries a default stylesheet (light and dark, following the system theme). The whole styling vocabulary is seven classes and eight tokens:

```
classes   card  stats  controls  primary  up  down  stale
tokens    --crow-bg  --crow-surface  --crow-text  --crow-muted
          --crow-border  --crow-accent  --crow-up  --crow-down
```

Use the classes as ordinary attributes — `div[..., class = 'card']`. To restyle, put plain CSS in `css/app.css` in the project; it is inlined after the defaults, and overriding a token (`:root { --crow-accent: #c60; }`) recolors every element and chart that uses it. `crow check` flags unknown `--crow-*` names with a did-you-mean. CSS never appears inside `.crow` files. Design: `style-module-design.md`. Where this is going — a closed layout vocabulary with an engine written in Crow, so tests can state where things are and what colour they have without a browser — is `ui-engine-design.md`.

### Core (Server)

```
use web, ui
use views.home

serve[3000]
page['/', home.render]
```

The transpiler manages the application instance implicitly — no `app` variable needed. `page[path, view]` registers a LiveView page rendered by a view; event handlers are bound inside the view (`click = inc`), so there is no separate handler argument. `serve[port]` starts the server (`serve[port, refresh = N, budget = M]` adds periodic re-rendering and a latency budget — see Background Work). The order in source doesn't matter: the transpiler always emits `serve[]` after all routes are registered.

If no routes are registered, `serve[]` shows a default welcome page so the user always sees something.

If the port is already in use, the server prints a friendly error: `error: port 3000 is already in use`.

| Function | Description | Example |
|---|---|---|
| `serve[port]` | Start the server on the given port (implicit app) | [Views: the whole element vocabulary](../by-example/views-elements.md) · [Web: responses and submodules](../by-example/web-more.md) · [Web: a JSON route](../by-example/web.md) · [View Blocks](../ch12-01-views.md) · [Reactive State and Events](../ch12-02-reactive-state.md) · [Components](../ch12-03-components.md) · [Slow Work and the Live Loop](../ch12-04-background-work.md) · [Testing a Live App](../ch12-05-testing-live-apps.md) · [Charts and Styling](../ch12-06-charts-and-style.md) · [Views and Live Updates](../ch13-03-views.md) |
| `page[path, view]` | Add a LiveView page rendered by `view` | [Views: the whole element vocabulary](../by-example/views-elements.md) · [View Blocks](../ch12-01-views.md) · [Reactive State and Events](../ch12-02-reactive-state.md) · [Components](../ch12-03-components.md) · [Slow Work and the Live Loop](../ch12-04-background-work.md) · [Testing a Live App](../ch12-05-testing-live-apps.md) · [Charts and Styling](../ch12-06-charts-and-style.md) · [Views and Live Updates](../ch13-03-views.md) |
| `route[path, handler]` | Add a route; `handler` is a named `fn handler[req]` that receives the request and returns a response (implicit app; `route[path, method, handler]` for other methods) | [Web: responses and submodules](../by-example/web-more.md) · [Web: a JSON route](../by-example/web.md) |

### Responses

| Function | Description | Example |
|---|---|---|
| `text[s]` | Plain text response | [Web: responses and submodules](../by-example/web-more.md) |
| `html[s]` | HTML response | [Web: responses and submodules](../by-example/web-more.md) · [Testing a Live App](../ch12-05-testing-live-apps.md) |
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
| `sign[claims, secret]` | Sign a JWT from a claims map | [Math: element-wise functions](../by-example/math-elementwise.md) |
| `verify[token, secret]` | Verify a JWT and return claims map |  |

### CORS (`use web.cors`)

| Function | Description | Example |
|---|---|---|
| `cors[app]` | Apply permissive CORS (allow all origins) |  |
| `cors[app, origins]` | Apply CORS with specific allowed origins |  |

### Sessions (`use web.session`)

| Function | Description | Example |
|---|---|---|
| `sessions[]` | Create an in-memory session store | [Web: responses and submodules](../by-example/web-more.md) |

### Templates (`use web.template`)

| Function | Description | Example |
|---|---|---|
| `templates[glob]` | Load templates matching a glob pattern |  |
| `render[templates, name, data]` | Render a template with data | [Views: the whole element vocabulary](../by-example/views-elements.md) · [View Blocks](../ch12-01-views.md) · [Reactive State and Events](../ch12-02-reactive-state.md) · [Components](../ch12-03-components.md) · [Slow Work and the Live Loop](../ch12-04-background-work.md) · [Testing a Live App](../ch12-05-testing-live-apps.md) · [Charts and Styling](../ch12-06-charts-and-style.md) · [Views and Live Updates](../ch13-03-views.md) |

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
| `ws[app, path, handler]` | Add a WebSocket endpoint | [Structured files: `use io`](../by-example/io.md) |
| `connect[url]` | Connect to a WebSocket server | [Databases: `use db`](../by-example/db.md) · [Working with Databases](../ch11-00-databases.md) · [Charts and Styling](../ch12-06-charts-and-style.md) · [Loading and Shaping the Data](../ch13-01-data.md) · [The Database Layer](../ch13-02-database.md) · [Views and Live Updates](../ch13-03-views.md) |
| `send[sock, msg]` | Send a message on a WebSocket |  |
| `recv[sock]` | Receive a message from a WebSocket |  |
| `close[sock]` | Close a WebSocket connection |  |

### LiveView / Pages

| Function | Description | Example |
|---|---|---|
| `page[path, view]` | Add a LiveView page (server-rendered with live updates via WebSocket) | [Views: the whole element vocabulary](../by-example/views-elements.md) · [View Blocks](../ch12-01-views.md) · [Reactive State and Events](../ch12-02-reactive-state.md) · [Components](../ch12-03-components.md) · [Slow Work and the Live Loop](../ch12-04-background-work.md) · [Testing a Live App](../ch12-05-testing-live-apps.md) · [Charts and Styling](../ch12-06-charts-and-style.md) · [Views and Live Updates](../ch13-03-views.md) |

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
