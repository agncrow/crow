# ui

Views and the element vocabulary, reactive state and inputs, the layout vocabulary, charts, and styling.


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

**Inside a view, the standard HTML element vocabulary is active** — plus the vocabulary's own elements: the containers `stack`, `row`, `grid`, `overlay`, `scroll`, the layers `modal`, `drawer`, `toast`, and `details`, `summary`, `dropdown`, `divider` (Revealing and framing, below) — `div`, `span`, `p`, `h1`, `h2`, `h3`, `h4`, `h5`, `h6`, `button`, `ul`, `ol`, `li`, `form`, `a`, `nav`, `header`, `footer`, `section`, `tr`, `td`, `th`, `thead`, `tbody`, `option`, `textarea`, `label`, `pre`, `code`, `input`, `img`, `table` — with their exact HTML names. **Outside a view, those names don't exist**: `p = 0.95`, `fn add[a, b]`, and `form = req.body` are ordinary code in any ui program, and `use ui` reserves nothing in your file. Calling an element outside a view is an error that points at the fix.

**A child that is a list contributes each item.** Markup a view or an element made is markup, anything else is text — so `rows = for s in sessions` over a view, then `stack[h2['Sessions'], rows]`, renders one row per session. This is how a page shows many of something with their own controls; `table[frame]` is for cells that are values. **A click can ask first**: `button['End session', click = end_session, confirm = 'End this session?']` shows the question in a dialog the runtime draws, and runs the handler only when the reader says yes. The server enforces it — the button offers the question, not the handler, and the dialog's own button offers the handler while the dialog is open — so the handler cannot be reached without the answer. On a table the question is a map from action to text: `actions = map[Delete = delete_session], confirm = map[Delete = 'Delete this session?']`. **Boolean attributes follow state**: `button['Start', disabled = starting]` writes `disabled=""` when true and nothing when false, for `disabled`, `hidden`, `readonly`, `required`, `checked`, `selected`, `multiple`, `autofocus`.

Views compose: a view calls other views (components) and ordinary functions freely. A module can export views — importers reach them qualified, like any module name:

```
--- cards.co
view card[title, body_text]
    div[h2[title], p[body_text]]

--- main.co
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
| `field[label, var, type = '', lines = 0]` | A labeled input two-way bound to reactive variable `var` (inside a view). `type` is the input's kind — `'text'`, `'number'`, `'date'`, `'time'`, `'password'`, `'file'` — and defaults to `'number'` for an int or float variable; `lines = 3` makes it a textarea | [Views: the whole element vocabulary](../by-example/views-elements.md) · [Reactive State and Events](../ch12-02-reactive-state.md) · [Testing a Live App](../ch12-05-testing-live-apps.md) · [Charts and Styling](../ch12-06-charts-and-style.md) · [The Element Vocabulary](../ch12-07-vocabulary.md) · [Views and Live Updates](../ch13-03-views.md) |
| `slider[label, var, min = 0, max = 100, step = 1]` | A range input bound to a number | [Views: the whole element vocabulary](../by-example/views-elements.md) · [Reactive State and Events](../ch12-02-reactive-state.md) |
| `checkbox[label, var]` | A checkbox bound to a bool | [The Element Vocabulary](../ch12-07-vocabulary.md) |
| `toggle[label, var]` | A switch bound to a bool — the same binding as `checkbox`, drawn as a toggle | [The Element Vocabulary](../ch12-07-vocabulary.md) |
| `radio[options, var]` | One of several strings, as radio buttons: `radio[['buy', 'sell'], side]` | [The Element Vocabulary](../ch12-07-vocabulary.md) |
| `tabs[options, var]` | One of several strings, as a tab strip; the view shows the selected tab's content with `if mode eq 'Practice'` | [The Element Vocabulary](../ch12-07-vocabulary.md) |
| `select[options, var]` | One of several strings, as a dropdown list | [Frames: `use frame`](../by-example/frames.md) |
| `progress[value, max = 1.0]` | A progress bar | [The Element Vocabulary](../ch12-07-vocabulary.md) |
| `icon[name, size = 'm', color = '']` | An icon from the closed set (below), sized by the type scale, colored by a token | [The Element Vocabulary](../ch12-07-vocabulary.md) |

Inputs are state: the reader's change lands in the variable, the handlers read it, and the next render draws it. There is no "value of the input" apart from the variable.

**Files in and out.** A file is an input like any other: `field['Trades', upload, type = 'file']` binds `upload`, declared `upload = map[]`. When the reader picks a file the page sends it to the server, and `upload` becomes a map of strings — `name` (the file's name on the reader's computer), `type` (its media type), `size` (its bytes, as text) and `path`, where the server keeps the copy — so a handler reads it as it reads any file: `csv[upload.path]`, `read[upload.path]`. The copy is the session's, kept under `.crow/uploads/` (a directory that ignores itself in version control) and removed when the session ends; a handler that keeps a file copies it (`cp[upload.path, …]`). A file bigger than the app allows — 10 MB, or `serve[…, uploads = 50]` in megabytes — is refused before it is sent, and the reader is told the limit. The other way, `download[value, name]` hands the reader's browser a file: a frame as CSV, a string as text, or a picked file (a map with a `path`) as that file's bytes, saved under `name`. It is an effect held to the handler's commit, as `notice` is, so a handler that stops sends nothing. In a test, `change[a, 'upload', map[name = 'trades.csv', type = 'text/csv', size = '120', path = 'fixtures/trades.csv']]` stands for a file the reader picked, and `downloads[a]` is the list of what the session was handed, each a map of `name` and `text`.

### Derivations — `derive`

Facts are stored; beliefs are derived. A derivation is a value computed from facts and data sources, read like a fact, never assigned, and recomputed only when something it read changed:

```
qty = 100
sym = 'IMRN'

derive total
    return qty * 2

derive trades[fresh = 300]
    return query[book, 'SELECT * FROM trades WHERE symbol = ?', [sym]]

view render[]
    return stack[p['total `total`'], table[trades]]
```

`derive name` opens a block with a `return`, like a view; it never takes parameters. The name reads as a variable anywhere a fact can be read — views, handlers, tests, other derivations — and assigning it is an error that says what to change instead. A derivation writes no fact: only handlers write.

**What it reads is decided once, from the source**, and both execution modes obey the same sets: the facts and derivations it names (transitively through the functions it calls, and inside interpolated strings), `now[]` if it or a callee reads the clock, and the tables its query literals read. A derivation recomputes, on its next read, when

1. a handler wrote a fact it reads;
2. a derivation it reads recomputed and its value changed — an equal number, string or bool cuts the chain (frames always propagate);
3. a statement in this program wrote a table it reads — an `exec`, an `insert`, or a `query` whose statement writes (`INSERT … RETURNING id` reads back the row it wrote, and the table is written all the same);
4. it reads `now[]` and a tick of `serve refresh` passed — a click's render is not a tick, so a click recomputes only what it moved;
5. it reads a table this program never writes — an external source — and its freshness window elapsed. `fresh = N` is seconds; the default `fresh = 0` means *as fresh as the loop*: recomputed every tick, which is what a query inside a view did before. Declaring `fresh` is the relaxation.

A derivation that fails — a query error — holds the error value; readers see `.ok` and `.reason` and a view renders the reason in place. `captured[x]` on a derivation is the moment it last ran. Derivations live in `use ui` programs with a page: they are reactive state; in the test harness `html[a]` is a tick and `click[a, h]` marks what the handler wrote. Design: `derive-design.md`.

**Who holds a derivation is inferred from what it reads.** A fact is a top-level variable; a database or socket handle is not one, and neither is a `fetch`/`poll` binding. A derivation that reads a fact a session writes — assigned by a handler or a helper a handler calls, or bound to an input like `field` or `select` — or another derivation held per session, is computed per session. One that reads only constants, tables and the clock — the day's universe, the latest bar in a table — is computed once for the app and shared by every session, and a table write in any session reaches every session's derivations over that table. `crow check` prints each derivation's read set and who holds it.

**A render never waits for a query it does not need.** A derivation that reaches out — its read set includes a table or an external source — is computed off the render in a served page: a render reads its latest value, and when that value is stale the runtime starts a recompute on a worker thread and the render goes ahead with what it has; the page renders again when the new value lands. Staleness has a cause, and the cause decides what waits. A derivation over a table the current event wrote is recomputed before that event's render, so a click's own writes are on the page in its answer — after `buy`, the blotter shows the trade. One that is stale because time moved, because a fact it reads changed, or because another session wrote its table shows its last value until the recompute lands. A handler that reads a derivation always gets the recomputed value; a derivation over facts and the clock alone is computed where it is read.

Before its first value, a derivation whose value can be an error — a query's frame, anything that can fail — reads as an error of kind `'pending'` with the reason `loading`, which a view renders in place like any failed read; one that cannot fail is computed where it is first read. One recompute of a derivation is in flight at a time for each holder — the session, or the app for a shared one, so readers arriving together run a shared query once — and a value whose inputs moved while it was computed is dropped and computed once more with the latest inputs. `running[x]` is true while a recompute of `x` is in flight. The root element of a view that read a value still being recomputed carries `data-crow-stale` while the value is behind something the reader did — it was computed before their last change (a click or an input that moved a fact), or there is none yet — and the default stylesheet dims it after 300 ms, so a fast recompute never flashes. A value behind only the clock is not marked: a query over a playing replay's play-head recomputes each tick and is always one query behind, where marking it dimmed every pane it feeds once a second. In the test harness every event settles before the next line runs: the handler's writes, then every recompute they made stale, so a test reads the page a reader sees once the data is in.

**A write reaches every reader.** When a handler writes a table, every session whose page read a derivation over that table renders again, recomputing it off the render: a reader sees another reader's change without waiting for a refresh tick, and without a line of code to publish it. A write made outside the program — another service, a person at a SQL prompt — is seen through `fresh` and the ticks, as above.

**Design Note: no channels.** Considered: `channel.join[name]`, `channel.send[name, message]` and an `on_channel` handler, so that a route, a job or another process could push to the pages that joined a channel (ui-framework.md, Server Push). Rejected: a write already reaches every reader. The database is the channel, and a page learns of another's change the way it learns of its own, by reading a derivation over the table; a channel would be a second way to say that something changed, and a message pushed past the database could say what the database does not. A process outside the program is seen through `fresh`.

**Rows keep their identity through `id`.** The live client patches the page in place: an element with an `id` is matched to the element with that `id` in the previous render, so a row rendered from data — `p['`s.name`', id = 's-`s.id`']` — moves when its list is reordered and is dropped when its item is gone, while its neighbours are untouched. Elements without an `id` match by position. A node that survives a render keeps its focus, caret and scroll, so the field a reader is typing in is never rebuilt under them.

**The checker teaches the form.** A top-level variable assigned from a query and read by a view (a launch-time snapshot), and a query against a remote database inside a view, are errors in `crow check`; `d` in the check loop rewrites the first into `derive x` and hoists the second above its view as `derive rows[fresh = 60]`.

### Tables over frames

**A big table pages.** `table[frame, page = 50]` shows fifty rows at a time, says which (`1–50 of 12000`) and offers the previous and the next page; the frame stays on the server and only the page shown is drawn, so a hundred thousand rows cost the reader one page. Each paged table keeps its place for its session: new rows arriving keep the reader on their page, and a frame that shrinks brings them to its last one. A table of more than 1,000 rows without `page` is refused, as a chart over its point budget is — in its place the view shows `table: 12000 rows — a table shows at most 1000 at a time: table[frame, page = 50]`, never a silent cut. A test sees the first page.

`table[frame]` renders a frame's rows. Numeric columns are typed: their cells carry the class `num` and the stylesheet right-aligns them with tabular figures. Six named arguments make it interactive and legible: `click = handler` sends the row's key to the handler when a row is clicked (`fn focus[symbol]`), `key = 'col'` names the column whose cell is sent (the first column by default; a numeric key travels as a number, a text key as a string), `mark = 'col'` names a column whose truthy cells put the class `marked` on their row, tinted by the stylesheet, `delta = ['pnl', 'pct']` names columns whose cells are colored by sign (`up` / `down`), `actions = map[Replay = replay, Delete = ask_delete]` adds a trailing cell of buttons to every row, each sending the row's key to its handler, and `hide = ['id']` keeps columns out of the table that stay in the frame — the key column, a helper. In the test harness `click[a, ask_delete[3]]` presses a row's button.

### Sliders

`slider[label, var, min = 0, max = 100, step = 1]` is a range input bound to a reactive variable, exactly as `field` binds a text input. A handler reads the variable when the reader acts (a `Seek` button beside the scrubber).

A bound input keeps its variable's type: text typed into a field bound to an integer becomes an integer (a float is truncated), a float variable gets a float, a bool variable gets a bool, and text that does not parse leaves a number alone. When a handler assigns to a bound variable, every field bound to it shows the new value on the next render; only the input the reader is typing in keeps its draft.

### Handlers and their helpers

A view sees reactive state, a handler moves it, and a function a handler calls that assigns a module-level variable is a handler too — its own part's, or one of a part it uses (`session.picked = symbol` in the scanner's part) — it joins the handler class in both renderers, so `fn restart[]` may call `fn rewind_to[t]` and the write lands in state. So does one that acts for the session — says a `notice`, signs the reader in or out, hands a `download`, sends `mail` — itself or through a function it calls, so what it does waits for its handler's commit. A pure helper (one that assigns no module variable) keeps its own scope. A helper a handler calls sees what the handler has written so far, and the handler sees what the helper wrote when it returns, in both modes — derivations included: a derivation the handler reads after a write to what it reads, its own or a helper's, is the recomputed one (`clock.jump[t]`, then `clock.head` is `t`). A helper that reads state and assigns none is a derivation, not a function: `derive band` computes the price band from the scanner's config, and the handlers read `band`. A function reading a derivation is refused with that advice — by `crow run`, `crow test` and `crow check` alike, which pass every program through the same checks before doing anything else. Reactive state may be an array, a map, a record or a list of records, or a frame; the compiled program keeps it as JSON between renders.

**A handler is a transaction.** It runs on a working copy of the session's facts, sees its own writes at once — as a helper it calls does — and when it ends its writes land as one; a render never sees a handler halfway, and two sessions never see each other's copies. Effects follow one rule, decided from the text: **an effect whose answer the handler uses runs when asked; one whose answer it does not use runs when the handler commits, in program order.** `r = query[…]`, `n = shell['wc -l x']` and `row = query[book, 'INSERT … RETURNING id']` answer at once; `notice['saved']` and `later[reload]` wait for the commit, and a stop drops them. A function's last line is its value, but a handler's value goes to nobody, so an effect on a handler's last line waits for the commit too, as does one a helper returns to a caller that calls it as a statement; a caller that reads the helper's answer (`r = notify[…]`) has it at once. Database writes run in a transaction: the first write a handler makes through a connection begins one on it, every later read and write on that connection is inside it, and it commits when the handler does and rolls back when it stops — `RETURNING` answers inside it. A handler that writes two databases commits them in program order; the language promises no atomicity across stores, and ClickHouse, which has no transactions, keeps a write the moment it is made.

**The notice.** `notice[text]` tells the reader something: an effect held to the commit, so a handler that stops never leaves a message saying it succeeded. The session keeps one notice, with the moment it landed; `notice` reads as a record with `text` and `at`, provided by the runtime like a derivation, so a view writes `toast[notice.text]`. A handler's stop is delivered the same way (`skip_symbol stopped: clickhouse query error: …`). A page that reads no `notice` gets the runtime's own toast, which fades after a few seconds; the program keeps no flag and clears nothing.

**Who the reader is.** `reader` is the page's signed-in reader: a map of strings, empty until someone signs in. A handler signs the reader in with `sign_in[record]` and out with `sign_out[]` — effects held to the handler's commit, like `notice`, so a handler that stops signs nobody in:

```
fn log_in[]
    found = query[book, 'SELECT id, role, hash FROM people WHERE name = ?', [name]]
    if len[found] eq 0
        notice['no one called `name`']
        return
    if hash[password, found.hash[0]] ne found.hash[0]
        notice['wrong password']
        return
    sign_in[map[id = str[found.id[0]], name = name, role = found.role[0]]]

fn log_out[]
    sign_out[]
```

Signing in sets `reader` for the session and gives the browser a cookie, so the reader is signed in on every page and tab they open next — after a restart too — for thirty days, or until they sign out. Signing out clears `reader` and the cookie, and ends that sign-in everywhere: every session holding it, another tab included, is signed out and redrawn at once, and the cookie is refused if it is ever sent again. A tab already open when someone signs in shows it the next time it opens the page. The program never sees the cookie: the runtime reads it when a page opens and fills `reader`. Nothing on the page can write `reader` — no input binds it and no handler assigns it; in a test, `change[a, 'reader', record]` stands for a reader who signed in. Authorization is ordinary code in the handler that needs it: `if reader.role ne 'admin'`. A signed-out reader has no `role`: reading it is an error, and a handler that meets one stops — nothing it did lands, and the reader is told why — so the check fails closed (in a test, `click` reports the error); `has[reader, 'role']` asks first. A derivation that reads `reader` is computed per session; a plain function is handed the reader as a parameter (`is_admin[reader]`), as it is handed a module variable. The record travels in the cookie, so it stays small: at most 1 KB — an id, a name, a role; look the rest up. The cookie is `HttpOnly`, `SameSite=Lax`, and `Secure` when the page came over https through a proxy (`X-Forwarded-Proto: https`). It is signed with the app's secret: `CROW_SECRET` (at least 32 characters) when set, otherwise a key the runtime writes to `.crow/keys/` on first use, a directory that ignores itself in version control. Changing the secret signs everyone out. Passwords are stored as `hash[password, 'password']` and checked with `hash[attempt, stored] eq stored` (### Hashing).

**A page for signed-in readers** checks `reader` in its view:

```
view orders[]
    if not has[reader, 'id']
        return sign_in_form[]
    return stack[
        table[open_orders, actions = map[Refund = refund]],
        button['Log out', click = log_out]
    ]
```

While the reader is signed out the page draws the sign-in form, and nothing else of the page reaches them: a session accepts only the events a render it sent offered, so `refund` is refused even in a message written by hand, and a derivation is computed only where it is read, so `open_orders` is never queried for them. When `log_in` commits, the same session draws the orders, at the same address, so nothing has to send the reader back. A page for one role checks the role the same way (`reader.role ne 'admin'`, after `has[reader, 'role']`).

**Design Note: no `auth` on a page.** Considered: `page['/admin', admin, auth = true]`, sending a signed-out reader to a login page and back once they sign in (ui-framework.md, Authentication). Rejected: the view's check already does all of it and fails closed. It draws the sign-in form in place, offers none of the page's handlers and runs none of its queries, and a second way to guard a page would leave two places to look for why a page is shown. The page keeps its address while the reader signs in, so there is nowhere to send them back to.

**A handler runs off the page's loop.** While it runs the page stays live — refresh ticks render and landed values render, from the committed facts — and `running[h]`, true while a `later` schedule of `h` is in flight, is also true while `h` runs as a handler: every element that offers `h` renders `disabled` with `aria-busy="true"`, so a button shows it is working with no flag in the program. Events are applied one at a time, in arrival order, bound inputs included: a change that arrives while a handler runs waits its turn, and the browser keeps showing what the reader typed. A second click on the handler that is running is dropped, as `later` skips a schedule in flight. `later[f]` is the one way to say the session's other clicks need not wait for this work (Design Note: *Handlers and later*).

**Design Note: no debounce.** Considered: `debounce = ms` on an input or a button, holding its events in the browser until the reader pauses (ui-framework.md, Debounce). Rejected: what it guards against is already kept small where it happens. A derivation over a field's fact has one recompute in flight, and a value its inputs moved past is dropped for the latest, so a search typed a key at a time runs at most one query at a time, always for the latest text; a second click on a handler that is running is dropped; and the events that arrive while a render runs are answered by one render. A delay the program chose would slow every reader to spare a server the runtime already spares.

### Views render wherever they are called

A `view` returns its markup from any call site — a `page[]`, the test harness, another view, or a program writing pages to disk (`doc.document[title, chapter[...]]`). **Text is text.** Markup is what a view, an element, a chart, an icon, a bound input, or `doc`'s `markdown`, `menu` and `pager` make; it stays markup however it travels — held in a local, collected by a `for`, put in a list, handed to another view as an argument. Every other string is text and is escaped, whatever it holds: a reader who types `<img src=x onerror=…>` into a field sees those characters on every page that shows them, never an image and never a script. A view given markup in one call and text in another is compiled once for each. Both renderers apply the same rule.

## Layout — the vocabulary

Pages are composed from layout primitives, never styled with CSS. Five containers join the element vocabulary inside views, and a fixed set of layout attributes applies to any element. Each has one written rule; the kernel table (`crow_runtime::layout`) turns it into the CSS a browser lays out with, and the layout engine — written in Crow, shipped inside the compiler — implements the same rule for tests and the desktop shell. Design and rationale: `ui-engine-design.md`.

| Container | Rule | Example |
|---|---|---|
| `stack[a, b, ...]` | children top to bottom, each the container's full content width | [Frames: `use frame`](../by-example/frames.md) · [Math: tensors](../by-example/math.md) · [Views: the whole element vocabulary](../by-example/views-elements.md) · [View Blocks](../ch12-01-views.md) · [Reactive State and Events](../ch12-02-reactive-state.md) · [Slow Work and the Live Loop](../ch12-04-background-work.md) · [Testing a Live App](../ch12-05-testing-live-apps.md) · [Charts and Styling](../ch12-06-charts-and-style.md) · [The Element Vocabulary](../ch12-07-vocabulary.md) · [Derivations](../ch12-08-derive.md) |
| `row[a, b, ...]` | children left to right on one line; overflow is a failure `fits[]` reports, never a silent wrap (`wrap = true` opts in) | [Views: the whole element vocabulary](../by-example/views-elements.md) · [Web: responses and submodules](../by-example/web-more.md) · [Charts and Styling](../ch12-06-charts-and-style.md) · [The Element Vocabulary](../ch12-07-vocabulary.md) · [Derivations](../ch12-08-derive.md) |
| `grid[a, b, ..., columns = 2]` | cells left-to-right, top-to-bottom, equal columns; `columns = '240px fill'` names tracks | [Views: the whole element vocabulary](../by-example/views-elements.md) · [Charts and Styling](../ch12-06-charts-and-style.md) |
| `overlay[base, top]` | `top` positioned over `base` by `anchor = 'center' \| 'top_right' \| ...` | [Views: the whole element vocabulary](../by-example/views-elements.md) · [Charts and Styling](../ch12-06-charts-and-style.md) |
| `scroll[child, height = '360px']` | a fixed-height viewport; the platform scrolls | [Views: the whole element vocabulary](../by-example/views-elements.md) · [Charts and Styling](../ch12-06-charts-and-style.md) |

| Attribute | Values | Example |
|---|---|---|
| `gap`, `pad` (`'m'` or `'m l'`), `margin` | the spacing scale `xs s m l xl` = 4 8 16 24 32 px |  |
| `align` | `start center end stretch` (cross axis) |  |
| `justify` | `start center end between around` (main axis) |  |
| `width`, `height`, `min_width`, `max_width` | `'240px'`, `'50%'` (width only), `'fill'` for remaining space; `aspect = '16/9'` |  |
| `size`, `weight`, `font` | the type scale `xs s m l xl xxl` = 12 14 16 20 24 32 px; `regular medium bold`; `font = 'mono'` sets the monospace face with tabular digits — a figure in a stat tile, a code-like value |  |
| `surface = 'card'`, `radius`, `border = true`, `shadow` | surfaces: radius `none s m l round`, shadow `s m l` |  |
| `color`, `background` | a colour token: `text muted accent up down surface bg` — never a literal colour |  |
| `lines = 2` | clamp a wrapping paragraph | [Structured files: `use io`](../by-example/io.md) |
| `below = 'phone' \| 'tablet'`, `becomes = 'stack'`, `columns_then = 1`, `hidden_below` | responsive rules at named widths (phone < 600, tablet < 1024) — no media queries |  |
| `open = true`, `side = 'left' \| 'right'` | a disclosure's or layer's state (any bool expression) and a drawer's or dropdown's side |  |
| `tip = 'text'`, `fit = 'cover' \| 'contain'`, `active = '/path'` | a tooltip on anything; how an `img` fills its `aspect` box; the current page of a `nav` |  |

Layout attributes are literals; a wrong value is a compile error naming the scale (`gap = 'huge' is not a spacing step (xs s m l xl)`), the same in both modes. Attributes outside the vocabulary (`class`, `href`, `name`, ...) pass through as HTML. Labels, headings, buttons, and cells are single-line; only `p` wraps.

### Revealing and framing

The vocabulary's own elements beyond the containers, each earning its place by a capability a stack of divs cannot compose:

| Element | Renders | Example |
|---|---|---|
| `details[summary[...], ..., open = true]` | A disclosure: the summary always shows, the rest only when open. `open` follows a bool — `open = show` toggles it from state | [Views: the whole element vocabulary](../by-example/views-elements.md) · [The Element Vocabulary](../ch12-07-vocabulary.md) |
| `dropdown[summary[...], panel, side = 'right']` | A button that opens a floating panel — its second child, usually a `stack` of buttons or links. `side` anchors the panel to the button's right edge | [Views: the whole element vocabulary](../by-example/views-elements.md) · [The Element Vocabulary](../ch12-07-vocabulary.md) |
| `modal[child, open = var]` | A dialog layer over a dimmed page while `open` is true; the child (a `stack` with a heading, text, and a `button['Close', click = close]`) is the dialog | [Views: the whole element vocabulary](../by-example/views-elements.md) · [The Element Vocabulary](../ch12-07-vocabulary.md) |
| `drawer[..., open = var, side = 'left']` | A side panel layer, sliding from the right by default | [Views: the whole element vocabulary](../by-example/views-elements.md) |
| `toast[..., open = var]` | A brief notice at the bottom corner while `open` is true; it fades on its own, and the program clears the flag | [Views: the whole element vocabulary](../by-example/views-elements.md) · [The Element Vocabulary](../ch12-07-vocabulary.md) |
| `divider[]` | A rule between siblings: horizontal in a `stack`, vertical in a `row` | [Views: the whole element vocabulary](../by-example/views-elements.md) · [The Element Vocabulary](../ch12-07-vocabulary.md) |
| `nav[a[...], ..., active = path]` | A navigation row; the link whose `href` equals `active` gets `aria-current="page"` and the stylesheet's active look | [Views: the whole element vocabulary](../by-example/views-elements.md) · [Reactive State and Events](../ch12-02-reactive-state.md) · [The Element Vocabulary](../ch12-07-vocabulary.md) |
| `header[...]` | The app's bar: a surface with a border holding its name, its `nav` and a status chip, laid out as a row (`gap`, `justify`, `align` as on any row) | [Views: the whole element vocabulary](../by-example/views-elements.md) · [The Element Vocabulary](../ch12-07-vocabulary.md) |
| `widget[name, props = value, on = map[event = handler]]` | A piece of the page the program's own JavaScript draws — a map, a rich editor, a calendar: the module `widgets/name.js` (below) | [The Element Vocabulary](../ch12-07-vocabulary.md) |

Three attributes any element takes: `tip = 'text'` is the browser's tooltip, `fit = 'cover'` (or `'contain'`) sizes an `img` into its box together with `aspect`, and `open` as above. The layers take no room in the flow and are hidden until opened; the layout engine lays an open layer out as a stack where it sits.

**One way out of the vocabulary: `widget`.** The vocabulary is closed, so a map, a rich-text editor or a calendar is a widget: `widget['map', props = map[lat = 51.5, lng = -0.1], on = map[pick = choose]]` draws the module `widgets/map.js` of the program's folder — plain JavaScript that exports `mount(el, props, send)` and, optionally, `update(el, props)`. The page loads the module once, calls `mount` with the element and the props (the value, as JSON), and calls `update` with the same element whenever a render changes them; the element's contents are the module's, and no render touches them. `send('pick', value)` runs the handler `on` names for `pick` — `fn choose[place]` receives `value` — through the offers like any click: a page offers exactly the events its widgets name, and a handler takes a widget's value as it takes a field's, so it checks what it is given. A module name is letters, digits, `_` and `-`; the server serves only `widgets/*.js`. In a test a widget is its element, and `click[a, choose['Paris']]` stands for what it would send. There is no other client code.

**The icon set**, closed like the rest of the vocabulary: `check x plus minus chevron_down chevron_up chevron_left chevron_right arrow_up arrow_down arrow_left arrow_right search menu more settings user home calendar clock refresh play pause stop upload download trash edit copy link external info warning error success star filter sort chart table bell mail lock eye eye_off dollar bookmark flag tag circle dot loader`. An unknown name is an error that lists them. Icons are inline SVG in the current text color, so `color = 'muted'` and the theme both apply.

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
| `x` | `'ts'` | Column of moments for the x axis | [Math: reductions and linear algebra](../by-example/math-linalg.md) · [Math: random, signal, special, and sparse](../by-example/math-more.md) · [Math: tensors](../by-example/math.md) · [Neural-network building blocks: `use math.nn`](../by-example/nn.md) · [Numbers and conversion](../by-example/numbers.md) · [Views: the whole element vocabulary](../by-example/views-elements.md) · [Testing](../ch09-00-testing.md) · [Charts and Styling](../ch12-06-charts-and-style.md) · [The Element Vocabulary](../ch12-07-vocabulary.md) |
| `candles` | `[]` | Four column names — open, high, low, close — drawn as candlesticks |  |
| `lines` | `[]` | Column names drawn as line series | [Structured files: `use io`](../by-example/io.md) |
| `bars_y` | `''` | Column drawn as a volume pane under the chart |  |
| `marker` | `0` | A moment marked with a vertical play-head |  |
| `tz` | `'UTC'` | Timezone for the axis ticks | [Frames: `use frame`](../by-example/frames.md) |
| `height` | `320` | Height in pixels |  |
| `panes` | `[]` | Lists of column names, each list drawn as its own strip under the volume with its own scale: `[['rsi'], ['cvd']]` |  |
| `levels` | `0` | Reference lines keyed by column, drawn dashed in the pane that holds the column: `map[rsi = [30, 70], cvd = [0]]` |  |
| `points` | `0` | A frame of markers on the main pane — columns `ts`, `y`, and optionally `label` and `kind` (`'buy'` an up triangle in the up color, `'sell'` a down triangle in the down color, anything else a dot) |  |
| `seek` | `0` | A handler: every bucket becomes a click target and the handler receives the bucket's moment — `seek = jump` with `fn jump[t]` |  |

Colors come from the style tokens (`--crow-up`, `--crow-down`, `--crow-accent`, and `--crow-series-2` … `--crow-series-6` for the second line onward, counted across the whole chart), so every chart is theme-correct with no color plumbing. Every line series is named in a legend at the top of its pane. The indicators a pane usually draws come from `use chart` (`### chart`). An empty frame renders a placeholder rather than an error; a frame over the point budget (1000 points) refuses with an error that says to aggregate first — coarser rows from the query, or `head[frame, n]` — never a silent downsample. The geometry and the SVG assembly live in `crow_runtime::chart`, so both modes emit identical markup. Design and roadmap: `chart-module-design.md`.

### Styling — the default stylesheet

Pages look right with zero CSS: every page shell carries a default stylesheet (light and dark, following the system theme). The whole styling vocabulary is seven classes and eight tokens:

```
classes   card  stats  controls  primary  up  down  stale
tokens    --crow-bg  --crow-surface  --crow-text  --crow-muted
          --crow-border  --crow-accent  --crow-up  --crow-down
```

Use the classes as ordinary attributes — `div[..., class = 'card']`. To restyle, put plain CSS in `css/app.css` in the project; it is inlined after the defaults, and overriding a token (`:root { --crow-accent: #c60; }`) recolors every element and chart that uses it. `crow check` flags unknown `--crow-*` names with a did-you-mean. CSS never appears inside `.co` files. Design: `style-module-design.md`. Where this is going — a closed layout vocabulary with an engine written in Crow, so tests can state where things are and what colour they have without a browser — is `ui-engine-design.md`.


Generated by `crow test --docs --write` from the language specification, section 11 and 13. Edit the spec, not this page.
