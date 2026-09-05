# The Crow layout engine — design

**Status: decided 2026-09-04; phase 2's first slice SHIPPED the same day.**
Built: the five containers and the layout attributes render in both modes
through one kernel table (`crow_runtime::layout`, emitting inline CSS +
`data-*` responsive rules); bad values are compile errors naming the
scale; the engine is written in Crow (`src/modules/layout.crow`, shipped
inside the compiler) and the harness reads it through `box[]`, `style[]`,
`fits[]`, and `app[view, width =]`. Not yet built: the font-metrics table
(text widths are a placeholder average), conformance fixtures (phase 3),
the desktop shell (phase 4), motion (phase 5), the `theme[]` declaration.
Rulings recorded below came from a long discussion with Vijay; the open
questions at the end are what the next round must answer.

## The problem

Crow can already test what a view *is*: the `app[]` harness renders a view
to a tree in a `test` block and `elements[]`, `change[]`, `click[]` state
truths about structure, content, attributes, and behaviour — no browser.

It cannot test how a view *looks*: whether the toolbar ended up beside the
block instead of under it, whether a card is the surface colour, whether
the stats row still fits at a phone width. Today those questions are
answered by the browser's layout engine, and only there.

Three constraints were set for any answer:

1. **No browser as a dependency** — not of the build, not of `crow test`.
2. **No network round trip for geometry.** Resize, scroll, hover must be
   local and instant; the LiveView loop is for *state*.
3. **One engine, written in Crow.** The same code lays out for tests, for
   the desktop shell, and — one way or another — for the web.

## The impossibility argument, and what it leaves

"How it looks" is defined by whichever engine draws the pixels. For an
HTML page that engine is the browser. A test is comprehensive only when it
runs the engine the user sees; any model of the browser is exact only
where the model and the browser implement the same specification and
approximate everywhere else. Therefore, **for arbitrary HTML and CSS,
comprehensive visual testing without a browser is impossible.** This is
not a tooling gap; it is a consequence of who owns the pixels.

The argument has one escape: make the styling surface *closed*, so that
"the same specification" is small, precisely written, and implemented
identically by every browser — and implement that specification in Crow.
Crow can do this because its UI vocabulary is already closed: a fixed
element list, seven classes, eight tokens, one shipped stylesheet, and no
CSS inside views. General web frameworks cannot, because their styling
surface is open. That asymmetry is the whole opportunity.

## Options considered

| Option | Why not |
|---|---|
| Headless browser in a `crow test --visual` tier (Playwright/Percy model) | Comprehensive for the web target, but a browser dependency. Rejected by ruling. Remains a legitimate *optional* end-to-end confirmation outside the test suite (the book runner already has such a script). |
| Lay out on the server, send a display list of positions; browser and desktop shell only paint | One engine, exact, DOM kept — but geometry then needs a round trip on resize (frame-rate messages hide it on a LAN, not on the internet). Rejected for constraint 2. |
| Same engine compiled to WebAssembly for local resize | Removes the round trip with no new compiler (Crow → Rust → wasm32). Rejected by ruling as an unwanted second build of the engine; recorded as the fallback if constraint 2 ever bites after the chosen design ships. Note: the roadmap's "client-side WASM: rejected" was about a client *framework*; a stateless layout function is a different thing. |
| A JavaScript backend for the engine | A second code generator whose agreement with the Rust one needs a JS runtime to test. Contradicts constraint 3. |
| Precomputed layouts at breakpoints | Instant and dependency-free, but approximate between breakpoints. Acceptable only as a transient, never as the answer. |
| Canvas rendering (Flutter-Web model) | Exact but discards the DOM: selection, accessibility, find-in-page, native inputs. Rejected. |
| External layout crates (taffy, yoga, Blitz/stylo) | taffy is a good CSS-subset engine and Blitz a real browser-less renderer; both are external dependencies and neither is Crow. Kept as reference implementations to *test against*, not to ship. |

## The design: CSS as the wire format, one algorithm in two places

1. **Crow defines a layout vocabulary of primitives** — not "a subset of
   CSS" but its own small set of containers and constraints (below). Each
   primitive has a written layout rule.
2. **The engine, written in Crow, implements those rules** and produces
   boxes: for every node, `x y width height` and resolved colours, given a
   viewport size.
3. **For the web, the engine emits CSS**, not positions: each primitive
   translates to a deliberately boring, unambiguous CSS form (block flow,
   flex row/column without intrinsic-size tricks, grid with fixed and
   fractional tracks). The browser lays out locally — resize, scroll, hover
   are instant, and the DOM stays real. This is what Crow does today; only
   the emitted CSS becomes the *serialization of the engine's rules*
   rather than a hand-written stylesheet.
4. **For the desktop shell, the engine emits positions directly**; the
   shell paints. It never needs a layout engine of its own.
5. **For tests, `app[]` runs the engine** and hands the boxes to the test
   block.

The engine and the browser agree because they implement one small,
precisely specified algorithm, the same way Yoga (React Native) agrees
with browsers for the flexbox subset it implements. That agreement is not
assumed; it is **conformance-tested with browser-derived fixtures that are
checked in as data** (below), so `crow test` never needs a browser.

Compared with the display-list design this trades "positions computed in
one place" for "one algorithm run in two places, conformance-tested". Under
the constraints, that is the better trade.

## The layout vocabulary

Customization is *composition plus tokens*, never free CSS — the
SwiftUI/Flutter/React-Native model. The vocabulary lives in the `ui`
module (not core), so it can grow under a lighter rule than the core budget.
Every entry below has three parts: the Crow form, the rule the engine
implements, and the CSS it emits for browsers. The rules are written so
that the emitted CSS is unambiguous across browsers; where CSS has a
famous ambiguity (intrinsic sizing under `flex-basis: auto`, percentage
heights without a definite parent), the vocabulary simply does not offer
the ambiguous form.

Existing elements (`div`, `p`, `h1`…, `table`, `button`, `input`…) keep
their HTML names and meanings; the vocabulary adds *containers and
attributes*, it does not rename HTML. Existing classes (`card stats
controls row primary up down stale`) become sugar over the primitives and
keep working.

### Containers

| Crow | Rule | CSS |
|---|---|---|
| `stack[a, b, c]` | children laid out top to bottom; each child's width is the container's content width; height is the sum plus gaps | `display:flex; flex-direction:column` |
| `row[a, b, c]` | children laid out left to right on one line; widths from each child's own sizing; overflow is an error `fits[]` reports, never a silent wrap | `display:flex; flex-direction:row; flex-wrap:nowrap` |
| `row[a, b, c, wrap = true]` | as `row`, but children move to the next line when the content width is exhausted; line height is the tallest child on that line | `flex-wrap:wrap` |
| `grid[a, b, c, d, columns = 2]` | children fill cells left-to-right, top-to-bottom; all columns equal | `display:grid; grid-template-columns:repeat(2, minmax(0, 1fr))` |
| `grid[..., columns = ['240px', 'fill']]` | named tracks: a fixed width and the remainder; `fill` shares remaining space equally among fills | `grid-template-columns: 240px minmax(0, 1fr)` |
| `overlay[base, top]` | children stacked in z-order at the same origin; the box is `base`'s box; `top` is positioned by `anchor` (`center`, `top_right`, …) inside it | `position:relative` on base, `position:absolute` + inset on top |
| `scroll[child, height = '400px']` | a fixed-height viewport over a child of any height; scrolling is the platform's, the engine only fixes the box | `overflow:auto; height:400px` |

### Sizing

| Attribute | Rule | CSS |
|---|---|---|
| `width = '240px'` / `height = '48px'` | fixed | `width`/`height` |
| `width = '50%'` | percent of the parent's content width (height percentages are not offered — the classic ambiguity) | `width:50%` |
| `width = 'fill'` | take remaining space in a `row`; several fills share equally | `flex:1 1 0; min-width:0` |
| `min_width`, `max_width` | clamp after the above | `min-width`/`max-width` |
| `aspect = '16/9'` | height from width | `aspect-ratio:16/9` |

Absent sizing means: in a `stack`, full content width; in a `row`, the
child's intrinsic width (text width, or its own children). Text intrinsic
width is measured from the shipped metrics table.

### Spacing and alignment

A scale, not pixels: `xs s m l xl` = 4 8 16 24 32 px (the tokens
`--crow-space-*`).

| Attribute | Rule | CSS |
|---|---|---|
| `gap = 'm'` | space between children of a container | `gap` |
| `pad = 'l'` (or `pad = ['m', 'l']` for vertical/horizontal) | inner space | `padding` |
| `margin = 's'` | outer space, only on a child of a `stack` (rows use `gap`; mixing margins into rows is what produces surprises) | `margin` |
| `align = 'center'` | cross-axis alignment of a container's children: `start center end stretch` | `align-items` |
| `justify = 'between'` | main-axis distribution: `start center end between around` | `justify-content` |

### Typography

A scale: `size = 'xs' 's' 'm' 'l' 'xl' 'xxl'` (12 14 16 20 24 32 px);
`weight = 'regular' 'medium' 'bold'`. Line height is fixed per size in the
metrics table, so a line's box is known without measuring.

| Rule | CSS |
|---|---|
| Labels, headings, buttons, table cells, card titles are **one line**: text that does not fit is cut with an ellipsis; the box is the line height | `white-space:nowrap; overflow:hidden; text-overflow:ellipsis` |
| `p[...]` **wraps**; its height is lines × line height, lines computed from the metrics table | normal wrapping |
| `p[..., lines = 2]` clamps a paragraph | `-webkit-line-clamp` |

This is the single decision that keeps most of a page's geometry
independent of text measurement.

### Surfaces and colour

| Attribute | Values | CSS |
|---|---|---|
| `surface = 'card'` | background `--crow-surface`, border `--crow-border`, radius `m`, padding `l` (today's `card`) | the card rule |
| `radius = 'm'` | `s m l round` | `border-radius` |
| `border = true` | one hairline in `--crow-border` | `border` |
| `shadow = 's'` | `s m l` | `box-shadow` |
| `color = 'up'` / `'down'` / `'muted'` / `'accent'` | text colour from the token set | `color:var(--crow-up)` |
| `background = 'accent'` | background from the token set | `background:var(--crow-accent)` |

Colour is only ever a token name. That is what makes `style[s, sel]` exact:
the engine resolves the same table the stylesheet was generated from.

### Responsive rules

Named widths, not media queries: `phone` (< 600), `tablet` (< 1024),
`desktop`.

| Crow | Rule |
|---|---|
| `row[a, b, c, below = 'tablet', becomes = 'stack']` | the row is a `stack` when the viewport is narrower than `tablet` |
| `grid[..., columns = 3, below = 'phone', columns_then = 1]` | column count changes at the named width |
| `hidden_below = 'tablet'` on any child | not rendered under that width — the engine removes it, the CSS hides it |

The engine takes the viewport width as an input (`app[view, width = 390]`
in tests), so every responsive rule is testable at every named width.

### Theming

The eight colour tokens exist today; the vocabulary adds the spacing and
type scales as tokens. An app overrides tokens in `css/app.css` (as now)
or, preferably, in `theme.crow` with a `theme[...]` declaration the engine
reads — the same values reach the stylesheet and the tests. A custom font
ships with the app and gets its metrics table generated at build time.

### Motion (deferred)

`transition = 'fade'` / `'slide'` on a child of an `overlay` or `stack`,
with fixed durations from a scale. Deterministic and small; not in the
first round.

### The dashboard in this vocabulary

The runnable version is `examples/layout_dashboard.crow`: `crow test` it for
the geometry and colour truths, `crow run` it to see the same rules as CSS.
The book walks through it in "Charts and Styling".

Today's `examples/dashboard.crow`, as it would read:

```
view stat_card[label, value]
    return stack[h3[label], p['`value`', size = 'xl', weight = 'bold'], surface = 'card']

view render[]
    trades = query[conn, 'SELECT symbol, qty, price, pnl FROM trades ORDER BY id']
    total_pnl = sum[trades.pnl]
    return stack[
        h1['Trading Dashboard'],
        grid[
            stat_card['P&L', fmt[total_pnl, decimals = 2, commas = true]],
            stat_card['Trades', len[trades]],
            columns = 2, below = 'phone', columns_then = 1
        ],
        row[field['Min qty', min_qty], button['Reset', click = reset_filters], gap = 'm', align = 'center'],
        h2['Trades'],
        scroll[table[trades], height = '360px'],
        gap = 'l', pad = 'l'
    ]
```

And the truths it makes possible, in the same file:

```
test layout[]
    s = app[render, width = 1280]
    cards = box[s, 'grid .card']
    assert[len[cards] eq 2]
    assert[cards[0].top eq cards[1].top]                   --- side by side
    assert[box[s, 'h2'][0].top gt box[s, 'row'][0].bottom]  --- heading below the controls
    assert[fits[s, 1280]]
    narrow = app[render, width = 390]
    cards = box[narrow, 'grid .card']
    assert[cards[1].top gt cards[0].bottom]                --- stacked on a phone
    assert[style[narrow, '.card'].background eq '--crow-surface']
```

Nothing in either block is CSS. Everything in the test is a property the
engine computes and the browser reproduces.

### What the vocabulary deliberately lacks

Absolute pixel positioning outside `overlay`, percentage heights, floats,
negative margins, `z-index` numbers, arbitrary selectors, pseudo-elements,
CSS animations, per-element font families. Each is either an ambiguity
generator or a second way to do something the vocabulary already does.
Needs that genuinely fall outside go into an unmodelled region (next
section).

## The modelled subset and the escape hatch

The engine claims exactness only inside its vocabulary. `css/app.css`
remains for token overrides and blessed classes. Anything beyond that is
placed in an explicitly marked **unmodelled region**: a view marks a
subtree as opaque with a declared size; the engine and the tests treat it
as a box of that size; `crow check` reports it as outside the model. This
is the honesty valve: developers are never blocked, and the guarantee is
never quietly stretched.

## Text metrics

Layout depends on advance widths, not on glyph rasterization. The engine
carries a per-character advance table for the kernel stylesheet's font
stack, measured once in Chrome and stored as fractions of the font size
(`src/modules/layout.crow`, `lay_sans_table` and friends). Two facts the
measurement surfaced and the tables encode: the system font has two
optical sizes with different advances — text below 20px, display from
20px — so `h1` uses a display table and everything else a text table; and
the stylesheet's `rem` is the root's 16px, not the body's 15px, so every
size, padding and margin constant in the engine is now derived from the
stylesheet at 16px. Summed advances land within 0.5% of a measured string;
the monospace stack is uniform at 0.602em. Small caps with letter-spacing
(`h2`, `th`, a card's `h3`) and the `h1`'s negative tracking are applied
before summing. Apps that ship their own font will regenerate the table
with `tests/layout/measure.py`'s sibling probe; the fallback for a glyph
outside the table is 0.55em.

## Conformance without a browser in the loop

`tests/layout/` holds fixtures: a Crow view tree, a viewport size, and the
boxes a real browser produced for the CSS the engine emits. They are
**generated once by an offline tool** (a browser is a dependency of the
person adding a layout feature, exactly as a compiler's conformance suite
is produced) and **checked in as data**. `crow test` runs the Crow engine
over every fixture and compares boxes within a stated tolerance
(sub-pixel rounding differs at boundaries; the tolerance makes it
explicit). Day to day, nothing in the build or the tests touches a
browser. The fixture generator pins its browser version.

Today `tests/layout/conformance.crow` holds two views — a dashboard (grid,
control row, overlay, scroll, cards) and a prose page (wrapping text,
three heading levels, a wrapping button row) — measured at 1280px and
390px by `tests/layout/measure.py` into `tests/layout/expected/*.csv`,
21 and 13 boxes per page. The tolerance is 6px on position and 8px on
size. Bringing the engine within it took four corrections, each a class of
error rather than a one-off: the 16px rem, margins outside the box (the
browser's convention, which the engine now shares), the descendant rules
of `.card` reaching the elements inside it, and the optical-size metric
tables above. CI runs the check on every push; a browser is needed only to
re-measure after a stylesheet or engine change.

Two parity rules follow the existing discipline:

- **interpreter == native**: the engine is ordinary Crow, so `crow test
  --parity` already holds its two executions to identical output;
- **engine == browser** on the fixtures, as above.

### The first conformance check, and what it found

The finch simulation page was the first real app rewritten in the
vocabulary (Sept 5 2026). Its `test layout_phone[]` passed — the engine
modelled `below = 'phone', becomes = 'stack'` on the control row and
reported `fits[s, 390]`. A real browser at a 390px viewport disagreed:
the row stayed horizontal and the page scrolled sideways to 466px. The
engine was right about the intent; the *emission* was wrong. The base
rule (`flex-direction:row`) is inline on the element, and a media-query
rule cannot beat an inline rule without `!important`. The responsive
rules now carry it, a kernel unit test pins that, and the browser agrees
with the engine again. That is precisely the class of defect the fixture
suite exists to catch: the engine and the CSS are two renderings of one
table, and only a browser can say whether the second one is faithful.

### The book as the second dogfood app

`use doc` (Sept 5 2026) puts Markdown rendering in the kernel:
`markdown[]`, `chapters[]`, `menu[]`, `document[]`, all pure, both
renderers calling the same Rust. `docs/book/site.crow` renders the whole
Crow Book with one view in the layout vocabulary and a `test layout[]`
block, and the interpreted and compiled exports are byte-identical
(attributes are emitted in name order for exactly that reason). Two
language rules came out of it: a view renders wherever it is called, not
only under `page[]` or the harness; and a view, like an element, renders a
failed argument as `error: reason` text instead of propagating it — a page
with one broken chapter is still a page. The in-page runner is a
Crow element too: `markdown[]` renders a runnable block as the editor,
gutter, toolbar row, output panel and expected output — kernel containers,
so `site.crow`'s `test runner[]` pins their placement — and `crow book`
renders and serves this site; the injected mdBook script and its Node test
are gone. Search (a kernel-built index over `plain[]`, filtered in the page), previous/next links (`pager[]`), and the phone menu followed; mdBook is deleted. Text metrics are measured advances now (see "Text metrics"), and `tests/layout/` holds the browser conformance fixtures.

## What developers see

In a `test` block, alongside the existing harness:

| Verb | Meaning |
|---|---|
| `app[view, width = 390]` | a session with a viewport (default: a desktop width) |
| `box[s, selector]` | matching elements' boxes: `x y width height` (records, document order) |
| `style[s, selector]` | resolved colours and typography as tokens: `background`, `color`, `size`, `weight` |
| `fits[s, width]` | true if nothing overflows its container at that width |

Truths then read: `box[s, 'button'][0].top gt box[s, 'h1'][0].bottom`,
`within[s, '.controls button', '.controls']`, `style[s, '.card'].background
eq '--crow-surface'`, `fits[s, 390]`. These join the mistake→truth table in
the book's "Testing a Live App".

## Phases

1. **Style truths** — `style[s, sel]` from the existing stylesheet's rules
   and tokens; no layout engine needed; catches "out of colour" completely,
   since views cannot carry inline CSS. Days.
2. **The vocabulary and the engine** — define the primitives, write the
   layout rules, implement the engine in Crow (`ui` module), `box[]`,
   `fits[]`, `app[view, width =]`. The engine emits the CSS the stylesheet
   used to hand-write. Weeks; the dogfood is the trading dashboard and the
   finch app.
3. **Conformance fixtures** — the offline generator, the fixture format,
   the tolerance, `crow test` integration. Days, after 2.
4. **The desktop shell** — a window and a painter over the engine's
   positions. Needs an OS window and a rasterizer (the same class of
   dependency as `wgpu`, which the runtime already carries); it lives in
   the shell, never in the engine or the tests. Weeks.
5. **Motion vocabulary**, unmodelled-region ergonomics, virtualized scroll
   regions — as real apps demand them.

## Open questions for the implementation round

- The exact primitive list and each one's written rule (the spec for
  the engine and for the emitted CSS).
- Scroll regions: sizing rules, and whether the browser's native scroll
  is enough (expected: yes) — virtualization later.
- Fixture tolerance and how rounding is specified in the rules themselves.
- The fallback-font policy and how a mismatch surfaces (a `crow check`
  finding? a runtime warning?).
- Whether `row`/`stack` collapse rules cover responsive needs without
  reintroducing media queries.
- Governance: the growth rule for the `ui` vocabulary versus the core
  budget rule.

## Why this is Crow-shaped

One way to do things: you compose boxes, you don't write CSS. A documented
rejection for every alternative. Verification that costs nothing: the
tests use the engine that produced the page. And the one asymmetry that
makes it possible — a closed vocabulary — is a principle Crow already had.
