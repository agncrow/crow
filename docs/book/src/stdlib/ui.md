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

**A child that is a list contributes each item.** Markup passes through, anything else is text — so `rows = for s in sessions` over a view, then `stack[h2['Sessions'], rows]`, renders one row per session. This is how a page shows many of something with their own controls; `table[frame]` is for cells that are values. **Boolean attributes follow state**: `button['Start', disabled = starting]` writes `disabled=""` when true and nothing when false, for `disabled`, `hidden`, `readonly`, `required`, `checked`, `selected`, `multiple`, `autofocus`.

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
| `field[label, var, type = '', lines = 0]` | A labeled input two-way bound to reactive variable `var` (inside a view). `type` is the input's kind — `'text'`, `'number'`, `'date'`, `'time'`, `'password'` — and defaults to `'number'` for an int or float variable; `lines = 3` makes it a textarea | [Views: the whole element vocabulary](../by-example/views-elements.md) · [Testing a Live App](../ch12-05-testing-live-apps.md) · [Charts and Styling](../ch12-06-charts-and-style.md) · [The Element Vocabulary](../ch12-07-vocabulary.md) · [Views and Live Updates](../ch13-03-views.md) |
| `slider[label, var, min = 0, max = 100, step = 1]` | A range input bound to a number | [Views: the whole element vocabulary](../by-example/views-elements.md) · [Reactive State and Events](../ch12-02-reactive-state.md) |
| `checkbox[label, var]` | A checkbox bound to a bool | [The Element Vocabulary](../ch12-07-vocabulary.md) |
| `toggle[label, var]` | A switch bound to a bool — the same binding as `checkbox`, drawn as a toggle | [The Element Vocabulary](../ch12-07-vocabulary.md) |
| `radio[options, var]` | One of several strings, as radio buttons: `radio[['buy', 'sell'], side]` | [The Element Vocabulary](../ch12-07-vocabulary.md) |
| `tabs[options, var]` | One of several strings, as a tab strip; the view shows the selected tab's content with `if mode eq 'Practice'` | [The Element Vocabulary](../ch12-07-vocabulary.md) |
| `select[options, var]` | One of several strings, as a dropdown list | [Frames: `use frame`](../by-example/frames.md) |
| `progress[value, max = 1.0]` | A progress bar | [The Element Vocabulary](../ch12-07-vocabulary.md) |
| `icon[name, size = 'm', color = '']` | An icon from the closed set (below), sized by the type scale, colored by a token | [The Element Vocabulary](../ch12-07-vocabulary.md) |

Inputs are state: the reader's change lands in the variable, the handlers read it, and the next render draws it. There is no "value of the input" apart from the variable.

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
3. an `exec` in this program wrote a table it reads;
4. it reads `now[]` and a tick of `serve refresh` passed;
5. it reads a table this program never writes — an external source — and its freshness window elapsed. `fresh = N` is seconds; the default `fresh = 0` means *as fresh as the loop*: recomputed every tick, which is what a query inside a view did before. Declaring `fresh` is the relaxation.

A derivation that fails — a query error — holds the error value; readers see `.ok` and `.reason` and a view renders the reason in place. `captured[x]` on a derivation is the moment it last ran. Derivations live in `use ui` programs with a page: they are reactive state; in the test harness `html[a]` is a tick and `click[a, h]` marks what the handler wrote. Design: `derive-design.md`.

**Who holds a derivation is inferred from what it reads.** A fact is a top-level variable; a database or socket handle is not one, and neither is a `fetch`/`poll` binding. A derivation that reads a fact a session writes — assigned by a handler or a helper a handler calls, or bound to an input like `field` or `select` — or another derivation held per session, is computed per session. One that reads only constants, tables and the clock — the day's universe, the latest bar in a table — is computed once for the app and shared by every session, and a table write in any session reaches every session's derivations over that table. `crow check` prints each derivation's read set and who holds it.

**Rows keep their identity through `id`.** The live client patches the page in place: an element with an `id` is matched to the element with that `id` in the previous render, so a row rendered from data — `p['`s.name`', id = 's-`s.id`']` — moves when its list is reordered and is dropped when its item is gone, while its neighbours are untouched. Elements without an `id` match by position. A node that survives a render keeps its focus, caret and scroll, so the field a reader is typing in is never rebuilt under them.

**The checker teaches the form.** A top-level variable assigned from a query and read by a view (a launch-time snapshot), and a query against a remote database inside a view, are errors in `crow check`; `d` in the check loop rewrites the first into `derive x` and hoists the second above its view as `derive rows[fresh = 60]`.

### Tables over frames

`table[frame]` renders a frame's rows. Numeric columns are typed: their cells carry the class `num` and the stylesheet right-aligns them with tabular figures. Four named arguments make it interactive and legible: `click = handler` sends the row's key to the handler when a row is clicked (`fn focus[symbol]`), `key = 'col'` names the column whose cell is sent (the first column by default), `mark = 'col'` names a column whose truthy cells put the class `marked` on their row, tinted by the stylesheet, and `delta = ['pnl', 'pct']` names columns whose cells are colored by sign (`up` / `down`).

### Sliders

`slider[label, var, min = 0, max = 100, step = 1]` is a range input bound to a reactive variable, exactly as `field` binds a text input. A handler reads the variable when the reader acts (a `Seek` button beside the scrubber).

A bound input keeps its variable's type: text typed into a field bound to an integer becomes an integer (a float is truncated), a float variable gets a float, a bool variable gets a bool, and text that does not parse leaves a number alone. When a handler assigns to a bound variable, every field bound to it shows the new value on the next render; only the input the reader is typing in keeps its draft.

### Handlers and their helpers

A view sees reactive state, a handler moves it, and a function a handler calls that assigns a module-level variable is a handler too — it joins the handler class in both renderers, so `fn restart[]` may call `fn rewind_to[t]` and the write lands in state. A pure helper (one that assigns no module variable) keeps its own scope. Reactive state may be an array, a map, or a frame; the compiled program keeps it as JSON between renders.

### Views render wherever they are called

A `view` returns its markup from any call site — a `page[]`, the test harness, another view, or a program writing pages to disk (`doc.document[title, chapter[...]]`). Inside an element, a string child that starts and ends with a tag passes through as markup; any other string is text and is escaped. Both renderers apply the same rule.

## Layout — the vocabulary

Pages are composed from layout primitives, never styled with CSS. Five containers join the element vocabulary inside views, and a fixed set of layout attributes applies to any element. Each has one written rule; the kernel table (`crow_runtime::layout`) turns it into the CSS a browser lays out with, and the layout engine — written in Crow, shipped inside the compiler — implements the same rule for tests and the desktop shell. Design and rationale: `ui-engine-design.md`.

| Container | Rule | Example |
|---|---|---|
| `stack[a, b, ...]` | children top to bottom, each the container's full content width | [Frames: `use frame`](../by-example/frames.md) · [Math: tensors](../by-example/math.md) · [Views: the whole element vocabulary](../by-example/views-elements.md) · [View Blocks](../ch12-01-views.md) · [Reactive State and Events](../ch12-02-reactive-state.md) · [Charts and Styling](../ch12-06-charts-and-style.md) · [The Element Vocabulary](../ch12-07-vocabulary.md) · [Derivations](../ch12-08-derive.md) |
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
| `size`, `weight` | the type scale `xs s m l xl xxl` = 12 14 16 20 24 32 px; `regular medium bold` |  |
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
| `nav[a[...], ..., active = path]` | A navigation row; the link whose `href` equals `active` gets `aria-current="page"` and the stylesheet's active look | [Views: the whole element vocabulary](../by-example/views-elements.md) · [The Element Vocabulary](../ch12-07-vocabulary.md) |

Three attributes any element takes: `tip = 'text'` is the browser's tooltip, `fit = 'cover'` (or `'contain'`) sizes an `img` into its box together with `aspect`, and `open` as above. The layers take no room in the flow and are hidden until opened; the layout engine lays an open layer out as a stack where it sits.

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

Use the classes as ordinary attributes — `div[..., class = 'card']`. To restyle, put plain CSS in `css/app.css` in the project; it is inlined after the defaults, and overriding a token (`:root { --crow-accent: #c60; }`) recolors every element and chart that uses it. `crow check` flags unknown `--crow-*` names with a did-you-mean. CSS never appears inside `.crow` files. Design: `style-module-design.md`. Where this is going — a closed layout vocabulary with an engine written in Crow, so tests can state where things are and what colour they have without a browser — is `ui-engine-design.md`.


Generated by `crow test --docs --write` from the language specification, section 11 and 13. Edit the spec, not this page.
