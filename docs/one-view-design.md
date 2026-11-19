# One view, four surfaces

A proposal, written 2026-11-15 from a conversation with Vijay. Nothing here
is built. The decisions in section 1 are Vijay's; the rest is the design
that follows from them, for review before any work starts.

## 1. The goal and the decisions

A developer writes a page once, as a `ui` view with its state and handlers.
The same page then works in four places with nothing lost:

| Surface | Renderer | Theme |
| --- | --- | --- |
| Desktop web | HTML | desktop |
| Phone | HTML | mobile |
| VR headset | Lory (WebGL), on a panel in the space | vr |
| AR headset | Lory (WebGL), over the room | ar |

Decisions made:

1. **One way to build an app:** `ui`. It stays in the language.
2. **No picturing.** A page is never turned into an image for a headset.
3. **No functionality lost in a headset.** Every control of a page works
   there, so Lory draws the page's own view, not a chosen subset.
4. **Each surface has its own theme.** A view states meaning; a theme
   decides the look.
5. **Updates are built properly:** a tick re-runs only the views whose data
   changed (section 3.4) and fast values skip the view entirely (3.5).
   Both are built, not one as a stand-in for the other.
6. **It goes ahead only if it performs** (section 4).

## 2. What exists today

Read in the code on 2026-11-15:

- **A view lowers to an HTML-shaped tree.** `crow_web/src/view/mod.rs`
  holds `CrowVNode`: tags (`div`, `table`, `tr`), attributes and children.
  `row`, `stack`, `field` and `button` are already HTML by the time the
  tree exists, so the tree cannot be handed to another renderer as it is.
- **Every event and every tick re-runs the whole page.**
  `crow_web/src/live/mod.rs` calls `served_render` for the page, compares
  the new tree with the last (`CrowVNode::changes`) and sends patches, or
  the whole HTML when the patches are not smaller.
- **Derivations already track what they read.** A `derive` has a static
  read set and recomputes only when something in it changed, with early
  cutoff (`derive-design.md` 3.2, 3.3). Views do not use this.
- **Lory's controls are semantic.** `packages/lory/gui.co` has
  `Node [tag, id, words, num, lo, hi, items, rows, kids, …]` and
  `Event [id, act, words, num]`, with `dark[]` and `light[]` themes. Its
  component modules cover forms, menus, tables, grids and pickers.
- **A widget is the one door to client code:** `widget[name, props, on]`.
  finchdash mounts its Lory charts and its scanner grid this way, each
  with a hand-written module and its own canvas.

A survey of the code (`one-view-survey.md`) then corrected four points:

- **The interpreter builds no tree.** Its views write HTML text directly
  (`src/ops.rs`, `render_element`); a tree exists only after that text is
  parsed again. The compiled program builds `CrowVNode`s, with raw HTML
  inside them for tables, icons, charts and markup.
- **The element renderer exists twice:** at run time in `src/ops.rs` and at
  compile time in `src/emit_rs/ui.rs`. Phase 1 makes it one.
- **The interpreter recomputes a derivation on every read** and copies the
  whole program state for each render and handler. The compiled program
  caches. Phase 2 gives the interpreter the same cache.
- **Early cutoff holds for scalar derivations only,** and children are
  matched by HTML `id`, not by a key of their own. Phase 2 needs both
  widened.

Tested on Vijay's Quest 3 the same day:

- finchdash's pages work in the Quest browser as ordinary windows.
- Its Lory charts draw there, flat, inside the page.
- The browser reports that both `immersive-vr` and `immersive-ar` can be
  entered.
- Not yet tested: whether a browser window stays visible while another
  window is in an immersive session.

## 3. The design

### 3.1 One semantic tree, two renderers

The view's tree keeps what each element *is*: its kind (`row`, `stack`,
`field`, `button`, `select`, `table`, `tabs`, `modal`, …), its properties
(label, value, bound fact, handler, disabled, tone, size) and its children.
Two renderers read that tree:

- **HTML:** what `CrowVNode` produces today. The output for an existing
  page must be the same bytes as before; the current tests hold that.
- **Drawn:** the tree as data for a Lory program, which builds Lory `Node`s
  from it.

The vocabulary is closed, so the mapping is a finite table, one row for
each element. An element with no drawn form is refused by `crow check`
when a page asks to be shown in a headset, with the element named. Three
things have no drawn form: raw HTML, a JavaScript widget, and anything a
page does only through CSS.

### 3.2 The drawn renderer

- **One Lory host program for a page.** It draws the page's panel and
  every drawn element (charts, grids, maps, scenes) as one program in one
  WebGL context, which is what an immersive session needs.
- **Events go the way clicks go.** A press on a drawn button sends the
  same event, with the same offer check, over the page's existing
  connection. The server cannot tell which renderer sent it.
- **The same session.** The page keeps running, hidden, while the headset
  shows the space. State, handlers, notices and confirmations are shared.
- **Typing** uses Lory's drawn keyboard in a headset.

### 3.3 Themes

A view names meaning and never appearance:

- tone: `primary`, `danger`, `muted`, `up`, `down`;
- size: `xs` to `l`; weight; gap; alignment;
- a role where a page needs one of its own (finchdash's chip and its
  position strip), declared once and given a look by each theme.

Four themes are sets of the same tokens:

| Theme | What differs |
| --- | --- |
| desktop | today's look |
| mobile | touch-sized controls, single column, larger text |
| vr | sizes in degrees of view, not pixels; larger targets; panel depth |
| ar | solid panels, strong contrast, no pure black on see-through displays |

A page's own CSS applies to the HTML renderer only. A page that depends on
CSS for meaning is a page to change (section 5, phase 6).

### 3.4 Partial re-run: a view is a derivation

A `view` gets what a `derive` has: a static read set and a kept result.

- A view re-runs only when a fact or derivation in its read set changed,
  or its arguments did.
- A view whose inputs are unchanged gives back its kept tree, and the
  compare skips it whole.
- Children in a list are matched by key, as they are today, so a row that
  moved is moved, not rebuilt.
- Both modes use the same read sets, so they behave the same.

Patches are made at the semantic level and each renderer applies them to
what it holds: the browser to its DOM, Lory to its node tree.

### 3.5 Live values

A value that changes on a clock or a feed (a price, a clock, a P&L) is
bound, not rendered:

- An element's text or attribute names a fact or a derivation directly.
- When that value changes, the runtime sends the value to the element. No
  view runs and nothing is compared.
- A value that moves with time alone (a play-head, a countdown) is given
  as a start and a rate, and the surface advances it itself, as
  finchdash's charts already advance their play-head.

Built in phase 3 for the first two (§7): the binding is found by the
compiler, not written — followed through a local assigned once and
through a parameter of a view that shows it only in bound places — and a
start and a rate are not built. A bound value is a text, a number of any
kind (a growing int and a decimal among them), a truth or a moment
(2026-11-16).

### 3.6 What never reaches the server

Scroll position, hover, the caret, a drag in progress and a panel's place
in the room belong to the surface.

### 3.7 The space

- The page stands on a panel. Each drawn element can stand on its own
  panel beside it (Lory's `window` package lays panels out).
- Sizes follow the vr theme's degrees of view.
- Entering and leaving keeps the session and returns to the same page.

## 4. Performance: what passing means

Proposed pass marks, measured on the Quest 3 with a compiled server:

| Measure | Pass |
| --- | --- |
| Frame time, 95th percentile, on finchdash's Execution page | 11 ms or less |
| Frames missed while scrolling a 200-row table | none |
| A press to the changed frame | 100 ms or less |
| A tick of one value: server work | 5 ms or less |
| A tick of one value: bytes sent | 1 KB or less |
| A playing Simulation session, 60 seconds | no missed frames from updates |

If phase 4's first page fails these, the work stops there and the design
is reviewed before any more of the vocabulary is drawn.

## 5. Phases

Each phase is finished, tested in both modes and measured before the next.

0. **Before anything:** the compiled build of a Myna app (in progress in
   another session), and a hook that runs when a page opens (`init`, which
   the docs describe and `page` does not take).
1. **The semantic tree** in `crow_web`, with the HTML renderer on top of
   it and every existing page's HTML unchanged.
2. **Views as derivations** (3.4), measured on finchdash's pages on the
   web. This speeds up HTML pages on its own.
3. **Live values** (3.5), measured on a playing Simulation session.
4. **The Lory host and the drawn renderer** with the vr theme. The
   Execution page first, measured against section 4; then the whole
   vocabulary, with a shared walk that drives both renderers through the
   same steps and compares state.
5. **The ar and mobile themes,** and drawn elements on their own panels.
6. **finchdash and Myna:** meaning moved out of CSS classes, the
   hand-written chart and grid modules replaced by the host, and Myna's
   pictured room retired.

## 5a. The tree as data

Built in phase 1 (2026-11-15). `crow_web::view::tree` holds the tree a
view makes (`Node`), its one builder (`element`, and `field`, `slider`,
`check`, `choice`, `select`, `progress`, `icon`, `widget`, `chart`,
`failed` for the rest), the HTML renderer (`html` for the markup,
`vnode` for the live runtime's nodes) and the data form (`to_json`),
which a drawn renderer reads. Both modes build the tree through the same
builder. The interpreter hands it its values. The compiled program hands
it Rust values of the same kinds (`tree::Given`): its emitted code for a
view calls `tree::element` and the control builders, and bakes nothing
in at compile time.

Every node is an object with a `node` field naming what it is:

| `node` | Fields |
| --- | --- |
| `element` | `kind` (as the view wrote it: `row`, `stack`, `button`, `p`, `table`, …), `options` (name to value, typed: text, number or truth, in name order), `events` (each `{on, handler, args?}`), `confirm` (the question a click asks first, when it asks one), `children`, and on a served page `binds` when the element holds a live value (phase 3, §7): each `{child: i, id}` (its `i`th child, a text or a control) or `{option: name, id}`, `id` the program's number for the binding |
| `rows` | a table's rows, under the `table` element that holds them: `frame` (`columns`, each `{name, numeric}`, and `rows`, the rows the table shows in the order it shows them, each a list of cells as data, a missing one `null`), `total` (rows in all), `click`, `key`, `mark` (when given), `delta`, `hide`, `actions` (each `{label, handler}`), `confirm` (label to question), `page` (`{size, index, at}`) and `sort` (`{index, by?, desc?}`) when the table is paged or sorted, and `refused: true`, with no rows, for more rows than a table shows unpaged |
| `control` | `kind`: `field` (`label`, `bind`, `value`, `type`, `lines`), `slider` (`label`, `bind`, `value`, `min`, `max`, `step`), `checkbox` and `toggle` (`label`, `bind`, `value`), `radio`, `tabs` and `select` (`options`, `bind`, `value`), `progress` (`value`, `max`), `icon` (`name`, `size`, `color`); a bound input given an `id`, a `class` or a `tip` carries each, by that name, beside these (not in `options`, which are a choice's), and one given none carries none |
| `chart` | what `chart[…]` was given: `frame`, `x`, `candles`, `lines`, `bars`, `marker` (`null` for none), `tz`, `height`, `panes`, `levels`, `points` (a frame or `null`), `seek` (a handler or `null`) |
| `widget` | `module`, `props`, `on` (event to handler) |
| `text` | `text` |
| `markup` | `html`: HTML that arrived as HTML (`doc.markdown`, `menu`, `pager`, a markup value), as it is |
| `failed` | `text`: what the page shows for a value that failed (an id the server's log has the reason under) |

What a renderer needs to send an event back is in the tree: an event's
handler and `args` are what the page sends (`{event: handler, value:
JSON array of args}`); a control's `bind` is the fact a change sets
(the page's `__field:bind`, or `__select:bind` for `select`); a table's
row click sends the row's `key` cell, an action its label's handler with
the same cell, a sorted header `__sort` and a pager `__page` with the
table's `index`. Nothing in it is HTML's: no tag, no class or style the
layout table makes, no `data-crow-*`. A markup node is the one place
HTML stands, marked as such.

Choices the documents had left open, taken so the HTML stays the same
and the tree says what the view wrote:

- An element's options are kept in name order: the interpreter has
  always read them as a map, and the HTML renderer sorts them anyway.
- `seek` on an element is dropped, as the page never wrote it; it is a
  chart's, and a chart node keeps it.
- A table's frame is a `rows` node among the `table` element's children,
  so an element written around it (`table[thead[…], frame]`) stays an
  element; the table's own options (`click`, `key`, `page`, …) move to it,
  and anything else on the table stays the element's.
- A node handed to a builtin, an operator or JSON is its markup, as it
  was before views were nodes; a node handed to a function of the
  program's, kept in a list or in a variable stays a node. A list a
  builtin made of nodes (`push[cards, card[x]]` keeps its item as it is;
  `join` of nodes is their markup) places them as before.
- A page's `title` and `meta` come out of the interpreter's tree as they
  come out of the compiled program's: a page whose markup the browser
  would repair had kept them in its body interpreted.

- A text that opens a `pre` or a `textarea` keeps its first newline, in
  both modes: the browser drops the first newline there, so the renderer
  writes one more, in the markup and in the page's nodes. The compiled
  page had always done this; the interpreter's page lost the newline.
- A class, written or computed, joins the element's own as the layout
  table joins them: the element's class, then each option's in name
  order (`row[…, class = c, surface = 'card']` is `row x card`). The
  compiled page had put a computed class last (`row card x`).
- An element takes every event the builder knows (`click`, `input`,
  `submit`, `change`), a handler named alone or a held call. A text is no
  handler (`click = 'inc'` wires nothing), as the interpreter had it; the
  compiled page had wired the text.

The live runtime computes the events a page offers from the tree
(`Offers::of_tree`): an element's written attributes, a table's clicks
from the kernel (`layout::frame_table_clicks`), a chart's seek bands
(`chart::seek_clicks`), a control's binding, a widget's handlers. Only
markup is scanned. A test holds this to the old scan of the page's markup
over three thousand random trees, and `CROW_CHECK_OFFERS=1` makes a
served page assert it too.

A table's rows and an icon are built as the page's nodes from the
kernel's parts (`layout::Part`), the same parts the markup is written
from. A chart's SVG is read once as a fragment. The page as a whole is
no longer written and read back to build the live runtime's nodes:
`whole` in `view/mod.rs` now takes tables and drawings as written too,
held by a test of forty thousand random trees against the reading.

What is still markup:

- `doc.markdown`, `menu`, `pager` and any markup value a program makes:
  they arrive as HTML, and the live runtime reads them back as the
  browser would.
- A node a compiled program keeps in a variable. The lowering types such
  a local as markup (`lower_ir.rs`, `full` and `declared_as`), so the
  compiled program holds the node's HTML there and hands it on as markup.
  The interpreter keeps the node.
- An element whose options name one attribute twice (a `tip` beside a
  `title`): its markup keeps both, as the page always had them.
- A page whose markup the browser would repair (a `div` in a `p`): it is
  read back as the browser reads it.

A drawn renderer's socket (phase 3, §7). A client that draws the tree
opens the page's socket as a page does (`/_ws/<path>`, with the page's
`p=` token or the tab's `t=`, or neither for a fresh session) and adds
`drawn=1` to its query. It is then sent the page as data in place of the
page's HTML, its patches and its path-keyed values; every other message
(`session`, `ack`, `head`, the address, a sign-in's, a download's, the
dev panel's) is what a page is sent, and the events it sends back are a
page's (`{"event": handler, "value": …, "seq": n}`, named in the tree).
An HTML page's socket is sent what it was, byte for byte.

- `{"type": "tree", "tree": <to_json of the page's body>}`: first, before
  `session`, with the page's `head`; again after any render that changed
  the page (whole, not as changes), and after the client sends
  `{"event": "__resync"}`. It carries `ack` and `head` as `html` does.
- `{"type": "values", "ids": {"<id>": value, …}}`: a values turn (no view
  ran), each binding computed again whose value is not the one last sent
  since the last tree, by its id in the tree's `binds`. A child's value
  is the node it places, as `to_json` writes it (`{"node": "text",
  "text": …}` or a control node), set as `children[at]` of the element
  whose `binds` holds `{child: at, id}`; an option's is the option's
  value, `null` where the option is left out. A binding reads only facts
  and derivations (a parameter it shows is its caller's expression), so
  an id placed in several elements holds one value at all of them.

`live::tests` `a_drawn_renderer_takes_the_tree_and_values_by_id` sets
each `values` message into the tree it was sent and compares it with
the tree a render makes, tick by tick, a structural move and a resync.

The interpreter reaches the tree through one name, `ops::tree`, as the
freeze test counts a compiler file's references to a module crate
(`src/freeze.rs`); its baseline was lowered for the files that now
reference fewer.

## 6. Risks and open questions

- **Read sets over-approximate.** A view that might read a fact counts as
  reading it, so some views re-run needlessly. Answered by phase 2 (§7):
  a view's derivations are recorded as it reads them, not
  over-approximated, and on finchdash the cost that is left is a page's
  root, which reads what its children read and runs at every event, in
  well under a millisecond.
- **The interpreter and derivations.** The interpreter had recomputed a
  derivation at each read. Fixed in phase 2: a served interpreted render
  and its handlers read them through the session's cache, as the compiled
  program does, and so does the test harness's interpreted page (§7). What
  is left is timing: the interpreter computes a stale derivation where it
  is read, where the compiled page shows the last value and catches up
  (`derive_bg`).
- **Live values are found, not written.** Phase 3 binds only what the
  compiler can prove is computed from scalar facts and derivations by
  pure builtins and the program's pure functions, and only where every
  read of that name in the view — through a local assigned once, or a
  view's parameter shown only in bound places — is such a place. A value
  a view's structure reads anywhere (finchdash's Simulation play-head,
  handed to widget props and to `in_trade = max[0, secs - opened]`,
  app/simulation.co) is structural, so its ticks still run the views it
  reaches. That errs safe and costs the render phase 2 already made
  cheap; a binding a developer writes would reach more.
- **Two renderers can drift.** The shared walk of phase 4 is the guard.
- **Large tables.** A drawn table of thousands of rows must draw only the
  rows in view; Lory's data grid does, a plain table may not.
- **Text entry.** Lory's drawn keyboard has no input-method support.
- **The Quest's windows.** If a browser window does stay visible beside an
  immersive session, a cheaper design exists for pages (the HTML window
  beside a drawn space). The test takes five minutes and should be run
  before phase 4.
- **Accessibility** is unchanged on the web, where pages stay HTML. A
  headset has no screen reader path in this design.

## 7. Built so far

**Phase 0 (2026-11-15).** `page[path, view, url = […], init = f]` and
`app[view, init = f]`: `f`, of no parameters, runs once as a session opens
the page, after the address facts are read and before the first render, as
a handler does. finchdash's Budgets page fills its guardrails with it. A
compiled test now keeps the computed facts an interpreted one keeps.

Left open from phase 0:

- A test's `app[v, init = f]` whose `f` takes parameters is refused when
  the test runs, not when it is checked.
- `later[…]` called from `init` on a first page load: whether its writes
  reach the page is unchecked.
- `init = 0` reads as no init.

**Phase 1 (2026-11-15).** The semantic tree, in two steps. In 1a the
interpreter's views became trees (`crow_web::view::tree`) with one
builder and one HTML renderer, the markup of every page unchanged. In 1b
the compiled program builds the same tree through the same builder; the
emitter's own element logic and its compile-time layout are gone
(`src/emit_rs/ui.rs`), with the `CrowVNode` helpers only it used. A
table's rows, an icon and a chart are the page's nodes, not markup read
back, and the events a page offers come from the tree. Section 5a lists
what is still markup.

What 1b changed in the HTML, each to what the other mode wrote:

- Interpreted: a `pre` or `textarea` whose text opens with a newline
  keeps it (the markup writes one more).
- Compiled: a computed class sits where the layout table puts it
  (`row x card`, was `row card x`).
- Compiled: a text given as a handler (`click = 'inc'`) wires nothing.
- Compiled: a `table` of a frame keeps its other options (`class`, `id`,
  layout); it had dropped them.
- Compiled: an icon whose name, size or colour the set lacks stops the
  render with the interpreter's reason; it had shown `error: …` in place.
- Compiled: a view outside a served page that holds a handler
  (`click = f`, `click = f[3]`) builds; it had not.

Left open from phase 1:

- The data form of a compiled file field holds no value (`null`); the
  interpreter's holds the picked file's record. The HTML is the same.
- A paged or sorted table in a view logged outside a served page counts
  from 0 for each table, compiled; the interpreter counts each one
  (the arguments of its pager and headers differ). Served pages agree.
- A compiled option of a type the builder does not read (a decimal, a
  moment) is left out, as the interpreter leaves it out; the compiled
  page had written it as text.

**Phase 2 (2026-11-15).** Views as derivations (3.4), in both modes.

- *What a view reads.* `derive::view_reads` (src/derive.rs), over the
  lowered program both modes run: the facts a view loads, itself and
  through every function and view it calls (transitively, handlers left
  out), and whether it reaches a builtin that answers by the run
  (`table::impure` but the console: `now`, `random`, `captured`,
  `running`, and the notice, whose rows are `Varies` now). The
  derivations it reads are recorded as it reads them, in order
  (`CrowLiveState::view_read_derive`, from every `derive_with` and
  `derive_bg` of a render), not taken from the source.
- *The cache.* `crow_web/src/live/views.rs`, in the session's state, one
  for both modes. A call is keyed by the view's name, its arguments'
  canonical text (compiled: their `Debug`; interpreted: `view_key`), and
  which call of that name and those arguments it is in the render. A
  view whose parameters include a function, a frame, a node or anything
  else that does not compare cheaply is not keyed and runs whenever its
  caller does, and so does a call whose arguments' text would pass
  `views::KEY_MAX` (4 KiB — a list of a thousand rows: writing and
  comparing it would cost about what running the view does, and the text
  is written only up to the limit, so refusing it is cheap). A call is handed its kept node when every fact's version
  is the one it saw, it starts at the same paged and sorted table
  numbers, every table it drew is on the page and in the order it was,
  and each derivation it read, brought up to date again in the order it
  was read, has the version it saw — a value recomputed equal to the last
  keeps its version (early cutoff), so its readers stay kept. A view that
  read the same facts and derivations takes the same branches, so the
  derivations it reads are the ones it read. A kept view moves the table
  numbers on, counts the calls inside it as running them would, and
  passes its reads to the view it stands in; a node marked stale is never
  kept, nor is a view a stale one stands in. Entries a render did not
  reach are kept within a budget (below). A handler's working copy and the
  app's state keep nothing.
- *Always run:* a view that reaches `now`/the clock, `random`/a draw,
  `captured`, `running`, `notice` (any `Varies` row or effect other than
  the console), and every view that calls one (the page's root, then:
  the views it calls are still kept). `crow check` does not print the
  reason yet; the spec says it at `ViewSpec.always`.
- *Shared nodes.* A subtree held by a kept view (an `Arc` held twice) is
  made into the page's nodes once and kept by the session
  (`tree::vnode_memo`); its nodes are `CrowVNode::Shared`, written to
  HTML once, checked as written once per kind of place, and the diff
  passes over the same one on both sides.
- *Early cutoff for non-scalars:* a list, a map, a record or a value that
  can fail of numbers, texts and truths compares all through (compiled:
  `==` where `Emitter::cuts_off`; interpreted: `same_value`). A frame, a
  tensor, a decimal, a moment or a parsed value always moves on.
- *The interpreter's derivations* are read through the session's cache in
  a served render (`IrInterp::served_read`), computed once per change, and
  a render thread keeps its copy of the program and loads only the facts
  whose versions moved (`served.rs::for_render`), where the template had
  been copied and every fact read from JSON at every render.
- *Keys.* `key = …` keys an element among its siblings (an `id` does where
  there is no `key`): the diff matches keyed children by key and moves
  the ones that moved (a `move` change, `CrowVNode::apply` and the page's
  script alike), and the page's morph matches by key too. Children with
  neither keep the old matching, so no page without keys changed.
- *Compiled node locals.* In a view, a local that holds a node and is read
  only where a node is placed (an element's child, the view's answer) is
  held as the node, not its markup (`Emitter::node_locals`); the
  lowering is unchanged.
- *The switch.* `CROW_VIEW_CACHE=off` runs every view; `check` runs every
  view, stops a render whose kept view would differ, and compares the
  shared page nodes with nodes made anew. `crow test --served` passes
  with it. The dev panel reports each render's views run
  and kept and its own time (`views`, `view_ms`).
- *Tests.* Served `38_a_view_nothing_moved_is_kept` (a `views` step in
  `.events` reads the dev panel's counts, both modes) and
  `39_keyed_children_move`; `tests/views_kept_harness.co` in both modes;
  `live::views::tests` (kept views, table numbers, and 400 random events
  against a page: every render's markup, offers and changes equal a full
  render's); `view::keyed_tests` (moves, shared nodes, 2,000 random keyed
  lists).

Measured (2026-11-15, one Mac, compiled; server time is the dev panel's
`render_ms`, press-to-change from sending the event to the first change
on the socket; medians, max in brackets).

A page built to show it — a ticking clock, a counter, and a 1,500-row
table:

| | event, before | event, after | tick, before | tick, after |
|---|---|---|---|---|
| compiled | 7.75 ms | 1.0 ms | 7.7 ms | 0.2 ms |
| interpreted | 8.15 ms | 1.0 ms | 7.7 ms | 0.4 ms |

(after, each render runs two views and keeps the table's.)

finchdash, a scratch copy on port 3911, before and after built from the
same source, three rounds each taken in turn (sorts, tabs, filters and a
search; patch bytes were the same, render for render):

| page | event, before | event, after | press, after | tick, before | tick, after | views run / kept at an event |
|---|---|---|---|---|---|---|
| /reports | 4.4 (48.8) | 4.7 (6.7) | 5.5 (7.9) | 2.6 (5.5) | 2.3 (3.8) | sort 8 / 7; tab 80–150 / 1–5 |
| /telemetry | 9.35 (72.0) | 9.85 (13.0) | 10.5 (14.2) | 4.5 (9.0) | 3.15 (5.4) | sort 12 / 11; filter 61 / 5 |
| /jobs-finch | 2.9 (4.9) | 2.7 (5.3) | 3.1 (6.1) | 2.05 (3.4) | 1.85 (3.1) | sort 9 / 7; search 49 / 5 |
| /scanner | — | — | — | 2.3 | 2.25 | tick 0 / 1 |
| /simulation | — | — | — | 1.75 | 1.55 | tick 0 / 1 |

Both pass marks hold before and after (press-to-change ≤100 ms, a tick
≤5 ms of server time at the median). finchdash's ticks change little
because its pages are small and a tick already ran one or two views'
worth of work; its events cost what they did because the view an event
moves is the one holding the big table (a sort runs the sorted table's
view and its callers, which is most of the time) or the top view of the
tab (a tab or filter: everything under it is new or handed new rows).
The first build of the after copy was a millisecond slower at an event:
views handed their whole trade list were keyed by writing the list out
at every call; a key now stops at 4 KiB (`views::KEY_MAX`) and such a
view runs, which is what brought it level.

**Phase 2, finished (2026-11-15).** What phase 2 had left open.

- *Handlers through the cache, interpreted.* A handler reads its
  derivations through the session's cache (`IrInterp::begin_handling`,
  `sync_served`, `served_read`), as a render does: a fact it writes is
  recorded by the store (`set_fact` into `FactWrites`), and before its
  next read of a derivation the written facts land in the working copy
  (`CrowLiveState::set`, so a value written back unchanged moves nothing)
  and the tables it wrote are noted, so the read sees the write and a
  derivation that reads neither keeps its value. The handler runs on the
  render thread's copy of the program with only the facts whose versions
  moved loaded (`served.rs::handle_event`, through `for_render`), where a
  copy had been made from the template and every fact read from JSON at
  every event; only the facts it wrote go back to the session, and under
  `CROW_VIEW_CACHE=check` every other fact is checked to hold what the
  session holds. `init` runs through the same path. A tick is a render,
  already on it; `later` runs inline in the interpreter, inside its
  handler. Tests: served `40_a_handler_reads_what_it_wrote`,
  `tests/handler_reads_harness.co` (both modes), `served::tests`
  `a_handler_reads_a_derivation_its_write_moved` (each derivation
  computed once per change, counted) and
  `handlers_through_the_cache_do_what_uncached_handlers_do` (300 random
  events — writes, reads after writes, a stop, a `later` — against a
  fresh copy of the program per event, as before: the facts, the stops
  and every page equal).
- *A view that pushes nodes onto a list.* The lowering types a list a
  node is pushed onto as a list of markup (`refine_pushed`, and `push`'s
  answer when its item is markup and the list's items are unknown or
  markup), and the emitter writes a node handed to a builtin as its markup
  (`table_call`), so it builds and shows what the interpreter shows. A
  list of text a node is pushed onto, or that a loop collects text and
  nodes into, is a list of text in both modes: the interpreter places a
  child whose type is text, or a list of text, as text (`held_as_text`,
  `as_text` in `ir_interp.rs`), as the compiled page had. That is a change
  to what an interpreted page showed for such a list (its markup had been
  placed as markup), and no test, app or example had one. Tests: parity
  `533_a_view_pushes_nodes_onto_a_list`, served
  `42_a_view_pushes_nodes_onto_a_list`.
- *The harness keeps views.* An interpreted test's `html[a]`, `click`,
  and an app's `init` run through a session state of the harness's own
  per app (`HarnessLive`, `harness_render`, `harness_transaction`), with
  the view cache and the derivation cache a served session has: the facts
  the test and its handlers moved (compared with what the state last saw,
  `same_fact`) and the tables written are landed before each render, so
  `CROW_VIEW_CACHE=off` and `check` work there as served. Every harness
  file passes in each of the three modes, unchanged. Tests:
  `ir_interp::harness_cache_tests` (views run and kept per `html[a]`; a
  handler's read after its write).
- *Tab switches.* Entries a render did not reach are kept for a later
  one (`views::views_end`): at most `KEEP_UNUSED` (1,024) of them, holding
  at most `KEEP_WEIGHT` (50,000) nodes between them (`tree::weight`, a
  table's rows counted one each), the least recently used let go first.
  A kept entry is checked as any is when it is reused — its facts'
  versions, its derivations, its tables' numbers — so one whose facts
  moved while it was away runs. A subtree an unused entry holds keeps its
  page nodes too (`vnode_memo` keeps what something else still holds).
  Tests: `live::views::tests` `a_tab_switched_back_to_keeps_its_views`,
  `unused_views_are_kept_within_a_budget`,
  `a_tree_past_the_weight_is_let_go_unused`; served
  `41_a_tab_switched_back_keeps_its_views` (the dev panel's counts, both
  modes).
- *Offers per kept subtree.* Measured first, with the walk timed inside `served_render`
  against the render's whole server time (one Mac, medians). On the
  synthetic 1,500-row page it is under 2%: 2.7 µs of 152 µs compiled,
  2.8 µs of 152 µs interpreted (a table's rows are one node to it). On
  finchdash it was not: /telemetry 301 µs of 10.9 ms at an event (2.6%)
  and 313 µs of 3.6 ms at a tick (8.9%); /reports 232 µs of 5.7 ms (4.0%)
  and 237 µs of 2.2 ms (10.0%). So what a subtree a kept view holds
  offers is read once while it is kept (`Offers::of_tree_memo`,
  `OffersMemo` in the session beside the page nodes' memo, chosen by the
  same rule: a child `Arc` something else holds too), and a render's
  offers hold the kept parts beside its own (`Offers::kept`; `allows`
  asks each, and two offers compare by what they allow). An entry holds
  its subtree weakly: a strong hold here and in the page nodes' memo had
  each kept the other's subtrees for good. After, the same rounds: a
  tick's walk 156 µs on /telemetry (3.1%) and 68 µs on /reports (3.2%);
  an event's stays where it was (257 µs, 2.5%; 246 µs, 4.6%), since an
  event's views ran and what they made is new. What is left is the walk
  of what the running views made — the page's root runs at every render
  — which no cache of kept subtrees reaches. `CROW_VIEW_CACHE=off` walks
  the whole tree; `check` walks it as well and stops a render whose kept
  offers differ. Tests: `live::views::tests`
  `a_kept_views_offers_are_read_once`, and the 400 random events, whose
  offers are compared with a full reading at every render.

finchdash /reports, a tab switched eight times per round, two rounds,
compiled, the same scratch copy and data (patch bytes equal, render for
render): server time 7.5 ms at the median before (max 10.8), 4.95 ms
after (max 7.6); press-to-change 8.8 ms before, 5.8 ms after. A tab
switched back to ran 127, 81 and 150 views before (Breakdowns, Overview,
Calendar) and 17–54, 46 and 6–43 after, keeping up to 45.

Left open from phase 2:

- The interpreter computes a stale derivation where it is read, where
  the compiled page shows the last value and catches up (`derive_bg`).
- A test body's own reads of a derivation, outside a handler and a page,
  are computed where they are read, interpreted.
- A list holding text and markup both shows its markup as text, in both
  modes; a list that could hold both kinds would need its items to say
  which they are, compiled.
- The offers of what running views made are walked at each render: on
  finchdash 2.5–4.6% of an event's server time, 3% of a tick's.
- finchdash's Overview tab still runs 46 views at each switch back; why
  those are not kept is not looked into yet.

**Phase 3 (2026-11-15).** Live values (3.5), in both modes. A turn in
which only values the views show moved runs no view: the page is the
last one with those values set, and they are sent as a `values` message.
Nothing new is written in a program.

- *The rule.* At the end of the lowering both modes share
  (`derive::bind_sites`, src/derive.rs), every element a `view` writes is
  searched for bound places: a child, or an option, whose type is a text,
  a number or a truth and which is computed from the session's scalar
  facts and derivations alone — constants, facts and derivations of those
  types, operators, spans (``'clock `t`'``), the int-to-float widening, list
  literals, and builtins that answer by their arguments alone
  (`table::impure` false, no `__` form, a scalar answer) — and reads at
  least one; or a bound control (`field` of any type but `file`,
  `slider`, `checkbox`, `toggle`, `radio`, `tabs`, a `select` of a list,
  `progress`) whose variable is a scalar fact and whose other arguments
  are such values (`tree::control_binds`). An option binds only where the
  page writes it as attributes alone (`tree::option_binds`: not an event,
  `confirm`, `seek`, `key`, `id`, `open`, `hidden`, `href`, `active`, nor
  an input's, option's or button's `value`, `type`, `name`, `checked` or
  `selected`); layout options are literals by the checker, so the options
  that bind are the others (`disabled`, `title`, `tip`, …). A widget's and
  a table's places are never bindings. A value that can fail is never
  one. Everything else is structural: a local, a parameter, a field, an
  index, a function of the program's, a builtin that answers by the run.
  A view's *bound* reads (`ViewReads.bound`) are the facts and
  derivations every one of whose reads, in the view and in all it calls,
  stands in a bound place; a name read once anywhere else is structural,
  as every read was in phase 2. A derivation is compared as phase 2
  compares it, so a derivation over a bound fact that a view reads
  structurally moves the turn to a render when, and only when, it
  recomputes to a new value.
- *The binding's number.* Each bound place's text (`child p <expr>` or
  `option disabled <expr>`) is numbered program-wide in text order, so
  both modes number it alike; the part keeps them (`ir::Part.bindings`,
  with each binding's inputs) and each element its own
  (`Expr::Tree.binds`). A served render places them on the tree
  (`tree::element_bound`; `Element.binds`, and `bound_within` on every
  element above one): a child only where it made exactly one text or
  control, an option only where the element kept it. `to_json` writes
  them (`binds`, 5a). A render outside a served page places none
  (compiled: `CrowLiveState::binding`; interpreted:
  `Served.render`).
- *A values turn.* `CrowLiveState::values_turn`
  (crow_web/src/live/views.rs), tried before every render of a served
  page that has bindings. The last render must have made the page from
  one view's node, unwrapped by a toast or a question (`views_page`).
  For every view that render placed: each fact it reads either holds the
  version it saw or is a bound read; each derivation it read, brought up
  to date once, either holds its version or is a bound read; its tables'
  pages and orders are where they were; and nothing read is still being
  computed. Then the bindings on the page whose inputs moved are computed
  again (`Values::eval`: compiled, the emitted `__crow_bound`; interpreted,
  `IrInterp::eval_bindings` on the render thread's copy, through
  `served::InterpValues`), every placed view's node takes them
  (`tree::rebind`, which copies only the path to what changed and keeps a
  subtree whose values compare equal), and each view's versions of what
  it read only in bindings move on, so the next render keeps it as if it
  had run. Any value that cannot be placed (a text that would now
  make no text, a layout that refuses it), any failure, or anything
  else leaves every view as it was and the turn renders. A turn whose
  nothing moved renders, as it did, keeping every view.
- *Addressing.* No id is written in the markup. The values turn's tree
  goes through the pipeline a render's does — the head split off, the
  shared page nodes, the offers — and is compared with the page the
  socket last sent (`CrowVNode::changes`), so the diff base is always
  what the page holds. Its changes are paths, as patches are, so they
  survive patches, keyed moves and kept subtrees as patches do. When
  every change is a text set or an element's attributes, the turn sends
  `{"type":"values","set":[…]}`, each entry `[path, text]`, `[path, text,
  in]` (the element the text stands in, as a patch's `in`) or `[path,
  tag, set, remove]` (`values_message`; an escaped text the markup
  writes is sent as the text it reads, and a text is set only where the
  page holds it as a node of its own — not an empty text, which the
  browser holds no node for, nor one beside another text, which it joins:
  `own_text`); otherwise it sends what a render sends, as before. The page applies each entry as the patch it is, and a
  path that does not fit asks for the whole page (`__resync`). A field
  being typed in keeps its draft, as with a patch.
- *Memos.* The page nodes' memo, the offers memo and each entry's node
  are left as a render that ran every view would leave them, and
  `CROW_VIEW_CACHE=check` holds it: after a values turn it runs the
  render too, and stops the session unless the changes between them are
  none and the markup, head and offers equal. A later full render, a
  reconnect (the tab's token) and a second session start from that.
  The dev panel counts the bound places set (`values`), and a values
  turn reports no view run or kept.
- *The switches.* `CROW_VALUES=off` turns values turns off (every turn
  renders, as in phase 2), to measure against; `CROW_VIEW_CACHE=off`
  turns them off with the cache.
- *Tests.* Served `43_a_bound_value_is_set_with_no_view_run` (a clock, a
  derivation, a progress bar, a computed `disabled`, a field, and rows
  that are structural; a `{"values": n, "sent": …}` step reads the dev
  panel's count and the message sent, both modes, with the cache on,
  checked and off); served `44_a_bound_text_grows_from_empty` (an
  empty text's first value comes as a render's change, a text with an
  `&` is set as the text it reads); `tests/values_harness.co` (both
  modes);
  `live::views::tests` `a_turn_that_moved_only_bound_reads_runs_no_view`
  and `values_turns_make_the_page_a_full_render_makes` (400 random events,
  over a hundred of them values turns: every page, its markup, offers and
  changes equal a full render's); `live::tests`
  `a_tick_of_bound_values_is_sent_as_values` (the socket: values, a
  structural move as patches, a reconnect by the tab's token, a second
  session) and the Chrome test `the_browser_applies_values` (ignored by
  default; a field's draft kept, a page knocked out of shape recovered).

What phase 3 changed in expected outputs: two served scripts whose turns
are now values turns, the dev panel's count only (the pages and patches
are unchanged). `38_a_view_nothing_moved_is_kept.events`: the three
counter steps read `{"values": 2}`, `{"values": 1}` and `{"values": 1,
"sent": …}` where they read views run and kept; its relabel step, a
structural move after them, still runs 3 and keeps 3.
`39_keyed_children_move.events`: both flips read `{"values": 1}` (the
flipped mark is read only in a binding).

Measured (2026-11-15, one Mac; server time is the dev panel's
`render_ms`, bytes its message size, per tick, medians, 90th percentile
in brackets; before is the same build with `CROW_VALUES=off`).

The synthetic page — a clock moved ten times a second by an event, a
derivation of it, a progress bar, beside a 1,500-row table:

| | server, before | server, after | views run / kept, before → after | bytes, before | bytes, after | message |
|---|---|---|---|---|---|---|
| compiled | 0.6 (1.0) | 0.6 (1.0) | 2 / 1 → 0 / 0 | 202.5 | 110 | patches → values |
| interpreted | 0.6 (0.8) | 0.6 (0.9) | 2 / 1 → 0 / 0 | 202.5 | 110 | patches → values |

Both pass marks hold before and after (a tick ≤5 ms of server time,
≤1 KB). Phase 2 had already taken the tick to two small views; phase 3
takes it to none and halves its bytes, and the server time is the
socket's turn, not the views'.

finchdash, a scratch copy on port 3911 (the paper-trading data; no
order, no write): its pages' idle ticks move nothing a view shows, so no
values turn happens there, and its Simulation, started and played by the
page's own Start and Play (page facts only), ticks the play-head through
a local handed to four views (`secs = clock_secs`,
app/simulation.co:1972), which is structural by the rule:

| page, tick | mode | server, before | server, after | views run / kept, after | bytes | values turns |
|---|---|---|---|---|---|---|
| /scanner, idle | compiled | 1.55 (3.6) | 1.55 (3.2) | 0 / 1 | 0 | 0 |
| /simulation, idle | compiled | 1.5 (2.9) | 1.7 (1.9) | 0 / 1 | 0 | 0 |
| /telemetry, idle | compiled | 2.55 (3.9) | 3.0 (4.5) | 0 / 1 | 0 | 0 |
| /simulation, playing | compiled | 3.45 (8.4) | 2.6 (7.6) | 12 / 14.5 | 178 | 0 |
| /scanner, idle | interpreted | 9.3 (10.4) | 9.2 (13.2) | 0 / 1 | 0 | 0 |
| /simulation, idle | interpreted | 8.05 (8.3) | 8.95 (9.1) | 0 / 1 | 0 | 0 |
| /telemetry, idle | interpreted | 7.6 (8.5) | 9.25 (10.0) | 0 / 1 | 0 | 0 |
| /simulation, playing | interpreted | 13.8 (16.6) | 20.6 (24.7) | 18 / 18 | 178 | 0 |

(Two to six ticks a run, so the differences between before and after
are noise: the same turns render in both. A playing tick changes two
texts, the clock and the recorded line; the views run because a local
holds the play-head. The interpreted idle ticks miss the 5 ms mark,
before and after: the time is the root's reuse check bringing the
page's derivations up to date, which a values turn does not change. An
earlier draft of this note said they passed it; they do not.)

**Phase 3, finished (2026-11-15).** What phase 3 had left open, in both
modes; nothing new is written in a program.

- *Values through locals* (`derive::locals_env`, src/derive.rs). A
  view's local assigned once at the top of its body (`assigned` counts
  every `Let`, set, path set and loop variable; a set in a branch or a
  loop counts too), whose type is a text, number or truth (or a list of
  them) and whose value is a bindable expression, stands for that
  expression: a bound place that shows it binds the expression, and a
  read of it anywhere else is a structural read of what the expression
  reads (`classify`, `name`). A local given a fact, a derivation or a
  field of one is that source's alias (`Env.srcs`). Anything else is a
  structural local, as before. A function of the program's that only
  computes (`pure_fns`: no store, effect, tree, function value, module
  call, impure or `__` builtin, and only such functions called) is a
  bindable call when its arguments are.
- *Values through parameters* (`derive::specialize`, run before
  `bind_sites` at the end of the lowering). A view's parameter that its
  body, and every view it hands it to, shows only in bound places
  (`binding_params`, a greatest fixpoint; a parameter read with another
  such parameter in one expression, set by the body, or of a view
  reached from itself, `looping_views`, is struck) is bindable. A call
  that hands such a parameter a bindable scalar of the same type gets a
  copy of the callee for that signature (`ir::site_name`,
  `view·siteN··`, its arguments' kinds in `ir::Fn.args`:
  `ArgKey::Bound(expr)`), in which the parameter stands for the caller's
  expression: its bindings compute that expression, so their ids are the
  program's and alike in both modes. The copy's key leaves the argument
  out; its text is kept beside the entry (`Entry.bound_args`). A kept
  entry handed a new value, or whose bound-only reads moved, is kept and
  its bindings computed again and set (`view_call`: `eval`,
  `tree::rebind`), as a values turn sets them; a value that cannot be
  placed runs it. Past 4,096 copies nothing is specialised.
- *Lists, frames and records from a source.* An argument of any other
  type that is a fact, a derivation, or a field or local alias of one is
  `ArgKey::Source`: the copy is keyed by the source's name and version
  (`CrowLiveState::source_version`; the caller read it, so its version is
  current) in place of its contents. Nothing else is fingerprinted: a
  frame or list computed in the view has no cheap identity (its
  contents are what a fingerprint would read), so it stays keyed by its
  text under `KEY_MAX`, or unkeyed. Each view's derivation reads carry
  whether they were structural where they were read (`Read.structural`),
  so a derivation read structurally by one view and only bound by another
  does not move the second to a render.
- *Idle ticks* (`CrowLiveState::settle`). Once a render (and once a values
  turn) the derivations that answer by the moment or by a source
  (`spec.now`, `spec.external`, noted as they are read) are brought up
  to date; then when the session's fact versions, the app's table writes
  (`TABLE_MOVES`) and the count of derivation values that moved anywhere
  (`DERIVE_MOVES`, in `compute_now`) are where the last settle found
  them, nothing any derivation reads has moved, and an entry last seen
  at that epoch is reused with no derivation of its brought up to date.
  A view that reads the clock or a draw still runs (`always`). The
  first tick after one that computed anything checks once.
- *Test bodies.* A test's own read of a derivation, interpreted, goes
  through its harness session's cache (`IrInterp::harness_read`), the
  facts the test wrote landing first, as the compiled harness reads it.
  A read at a new moment (the test's `change[a, 'now', …]`) starts a new
  tick of the session first (`IrInterp::harness_moment`), so a
  derivation that answers by the clock is read anew.
- *A drawn renderer's socket* (5a): `drawn=1` on the socket; `tree`
  whole on connect, on any render that changed the page and on a
  resync; `values` by id on a values turn. The tree is kept only for
  such a socket (`CrowLiveState.drawn`), so an HTML page's memos and
  bytes are what they were.
- *Table cells, widget props, markup* (decided). A table's cells are
  data, not places: a table's children are its rows, which the page
  nodes' memo already compares (`tree::element_binds` keeps `table`
  out). A widget's props stay structural: the widget protocol has an
  update path (`update(el, props)` when `data-crow-props` changes), but
  it takes the props whole, so a bound prop would be the whole props map,
  which no binding is (it is not a scalar), and the props finchdash's
  charts take are specs built from frames, which no rule could bind;
  binding one key would need a new bound place on widget nodes for no
  page measured. Markup stays markup: it arrives as HTML and is read back
  as the browser reads it.
- *Switches.* `CROW_FOLLOW=off` (lowering) leaves every local and
  argument structural, as before; `CROW_SETTLE=off` checks every kept
  view at every render, as before. Both exist to measure against.
- *Tests.* `derive::tests` `a_local_assigned_once_is_followed_to_its_value`,
  `a_scalar_shown_only_in_bound_places_is_a_bound_argument` (copies,
  structural, mixed and self-reaching parameters),
  `a_list_from_its_source_is_keyed_by_the_source`; served
  `45_values_through_locals_and_parameters` (a clock through a local, a
  pure function and two levels of parameters, a structural parameter,
  a tick as a render then as a values turn, both modes, on, check and
  off); `harness_cache_tests`
  `a_view_keyed_by_its_sources_version_is_never_stale` (300 random
  writes, each page through the cache equal to every view run anew,
  the lists' views kept), `a_test_bodys_own_reads_go_through_the_cache`;
  `live::views::tests`
  `an_idle_tick_brings_up_to_date_only_what_answers_by_the_moment`;
  `live::tests` `a_drawn_renderer_takes_the_tree_and_values_by_id`.

Expected outputs changed, each where fewer views run:
`41_a_tab_switched_back_keeps_its_views.events`, its last step ran 1
and kept 1 where it ran 2 (the tab's fact is read only in bound texts,
so the tab switched back to is kept with them set; its comment says
so); `harness_cache_tests` `the_harness_keeps_views_as_a_served_session_does`
(0 run, 1 kept, where 2 and 2) and `a_harness_handler_reads_what_it_wrote`
(2 and 2, where 4 and 0), for the same reason. No page, patch or
markup changed.

Measured (2026-11-15, one Mac, busy: other servers and an indexer
running, so medians move ±20% between runs of one build; before is the
same build with `CROW_FOLLOW=off CROW_SETTLE=off`; finchdash compiled
pages are debug builds, as `crow run` builds them).

The synthetic page — a clock moved ten times a second by an event, read
into a local, halved, and handed to a child view's parameter, a
progress bar, beside a 1,500-row table — 50 ticks a run:

| | server, before | server, after | views run / kept, before → after | bytes, before → after | message |
|---|---|---|---|---|---|
| compiled | 0.6 (1.0) | 0.6 (1.0) | 3 / 1 → 0 / 0 | 292 → 153 | patches → values |
| interpreted | 0.6 (1.1) | 0.6 (1.0) | 3 / 1 → 0 / 0 | 292 → 153 | patches → values |

(With every view run, `CROW_VIEW_CACHE=off`, a tick is 0.6 ms and four
views, 292 bytes: the table's rows are data, so the views cost little
here, and the time is the socket's turn.)

finchdash, a scratch copy on port 3911 refreshed each second (its
`refresh = 10` set to 1 in the copy, for thirty ticks and more a run;
paper data, Start and Play only, which write page facts):

| page, tick | mode | ticks | server, before | server, after | views run / kept, before → after |
|---|---|---|---|---|---|
| /simulation, idle | compiled | 113 / 122 | 2.3 (4.6) | 2.0 (3.7) | 0 / 1 → 0 / 1 |
| /scanner, idle | compiled | 95 / 115 | 2.9 (5.6) | 2.4 (4.8) | 0 / 1 → 0 / 1 |
| /telemetry, idle | compiled | 84 / 82 | 4.0 (6.9) | 3.7 (6.8) | 0 / 1 → 0 / 1 |
| /simulation, playing, a tick that moved | compiled | 43 / 41, twice | 5.4 (8.2), 5.4 (7.8) | 6.0 (8.3), 4.9 (7.3) | 17 / 18 → 13 / 13 |
| /simulation, playing, a tick that did not | compiled | 58 / 74 | 1.1 (2.4) | 1.4 (2.3) | 0 / 1 → 0 / 1 |
| /simulation, idle | interpreted | 41 / 41 | 13.4 (13.9) | 13.3 (15.0) | 0 / 1 → 0 / 1 |
| /scanner, idle | interpreted | 41 / 41 | 14.1 (15.0) | 11.7 (15.1) | 0 / 1 → 0 / 1 |
| /telemetry, idle | interpreted | 41 / 41 | 14.0 (14.5) | 13.7 (14.2) | 0 / 1 → 0 / 1 |
| /simulation, playing, a tick that moved | interpreted | 40 / 40 | 23.8 (25.2) | 17.3 (22.1) | 18 / 18 → 14 / 13 |

An idle compiled tick settles in 0.05–0.2 ms (nine timed derivations
brought up to date, nothing else moved) and passes the 5 ms mark; most
of what is left is the turn around the render. An interpreted idle tick
does not pass it, before or after: its time is the interpreter's render
itself, not the check (the check is skipped; the timed derivations it
brings up to date are computed where they are read, `derive_bg` being
the compiled program's alone). /reports' tab switches, compiled,
interleaved runs: 10.95 and 8.1 ms before, 11.55 and 9.1 after, with
following alone off 10.4 and 7.8: the same views run (Overview 46,
Breakdowns 54, Calendar 6–43), and following costs about a millisecond
a switch there, not looked into further (the likeliest: a copy whose
sources are keyed now also writes its other arguments' text, up to
4 KiB, before finding no key).

What still runs on a playing Simulation tick (compiled, 13 views), and
the read that runs each:

- `render`, `_body`, `_session`, `_session_over`: `clock_secs`,
  structurally. `clock_secs` is a `Big` (`floor[…]` answers one,
  app/simulation.co:205), and a binding is a text, an int, a float or a
  truth (`derive::scalar`, src/derive.rs:1504); and `_session_over`
  hands `secs` to widget props (`charts_left`, `charts_right`,
  `grid_charts`) and to `_position_strip`'s `in_trade = max[0, secs -
  opened]` by way of `_ticket`, each structural.
- `_play_bar`: handed `secs`, a `Big`, so no bound argument; its key
  moves (`new key`).
- `_scanner_card·site1··`, `_grid_card·site1··`: copies keyed by their
  sources, handed `secs` as a value: a new key each tick.
- `_symbol_card·site1··`, `_lead_card·site1··`, `_scanner_rows`,
  `_blotter`, `_blotter_table`, `workbench.screen`: unkeyed — handed a
  computed frame or list past 4 KiB (`rows`, the session's fills) or a
  node (`screen`'s body).

What runs on a switch back to /reports' Overview (46 views): `render` and
`_body` (the `tab` fact); `_tab_body·site1··`, `_overview_tab`,
`_charts_card`, `_series_card`, `_open_positions` and their
small views (`_kpi`, `_big`, `_toned`, `_cell`, `_hint`, …, new keys):
handed frames and records `_body` computes from its derivations
(`reports.within`, `reports.summarize`, `measures.by_close`), which have
no source and no cheap identity; `_header` (unkeyed, its arguments
past 4 KiB); `workbench.screen` (a node). Myna's `side_link` and
`_app_link` run for a reason of phase 2's: the count of tables drawn
before them differs between the tabs (`drawn_at`).

**Phase 3, two left-open items closed (2026-11-16).** In both modes;
nothing new is written in a program.

- *A growing int is a binding* (`derive::scalar`, src/derive.rs). A
  binding's value may be a growing int (`Big`: `floor[…]`, `round[…]`,
  an int past 64 bits), a decimal or a moment, as well as a text, an
  int, a float and a truth, wherever those bind: as a bound child or
  span, through a local assigned once, and as a bound scalar parameter
  (its specialised copy, `ArgKey::Bound`). An int widened where a
  growing one, a decimal or a fraction is taken (`Convert::ToBig`,
  `ToDecimal`, `ToFraction`, which cannot fail) is followed as the
  int-to-float widening is. That widening, not the play-head's type, is
  what had kept finchdash's play-head structural: the width pass holds
  `clock_secs` as a 64-bit int, and `store.clock[secs]`, whose parameter
  is a growing int, is handed `big(secs)`. The text is the render's in
  each mode, since a binding runs the code the view's own place runs
  (compiled `given_child`, interpreted `ops::given_child`); parity 534
  holds the two modes' texts alike, a negative growing int and one past
  64 bits among them. Decided beside it: a bound control's variable
  stays a text, an int, a float or a truth (`control_scalar`: the page
  sends its value back into the fact); a fraction binds but every way of
  making one from ints can fail (`fraction[a, b]` is a value that can
  fail), so in practice only fractions widened from ints or decimals
  bind; an option given a growing int, a decimal or a moment is left out
  by the builder in both modes (`tree::opt_of`), as a render leaves it;
  a truth already bound; Crow has no duration kind of its own (a
  difference of moments is a number of seconds).
- *Tests.* Served `46_a_big_value_is_bound` (a growing int from
  `floor[…]`, one past 64 bits, a decimal, a moment, an int widened into
  a function that takes a growing int, through a local and a parameter:
  each tick a values turn of 10 places, no view run, the `values`
  message held; on and with `CROW_VIEW_CACHE=check`); parity
  `534_a_growing_int_shows_as_a_bound_place_shows_it`; `derive::tests`
  `a_big_a_decimal_and_a_moment_bind_as_an_int_does`;
  `tests/values_harness.co` shows a growing int beside its clock. The
  served runner now fails a script that is no JSON list of steps (it had
  driven none, and passed).
- *Following on /reports' tab switches costs nothing measurable.* The
  millisecond noted below came from runs one after another, which on
  this Mac differ by up to twice between identical builds (four
  sequential runs today, two with following on and two off: 4.6, 8.35
  ms on; 8.1, 8.05 ms off). Measured instead with two servers of the
  scratch copy up at once, one built with `CROW_FOLLOW=off`, each switch
  sent to one and then the other in alternating order (64 switches a
  run, three runs, paired by switch): the median of following on minus
  off was +0.4, −0.3 and −0.05 ms, its sign following which port held
  which build, not the setting. Timed by phase (a temporary timer around
  the values turn tried first, the views' render, and the page's nodes
  and offers), on and off were alike to within 6%: 0.33 / 0.34 ms,
  4.68 / 4.67 ms and 1.40 / 1.46 ms in one run, 0.33 / 0.31, 4.48 /
  4.74 and 1.34 / 1.40 in another (the same 46 views ran and 6 were
  kept in each). The tried values turn, 0.3 ms of each switch, is the
  same either way; checking every kept view's facts before any of its
  derivations was tried, and changed nothing (0.33 ms), so it was not
  kept.

Measured (2026-11-16, one Mac; server time is the dev panel's
`render_ms`, medians with the 90th percentile in brackets; before is
the same tree with `derive::scalar` as it was).

The synthetic page — a clock moved ten times a second by an event,
`floor[…]` of it (a growing int) shown as a child, halved through a
local, and handed through a parameter into a function of the program's
that takes a growing int, a progress bar, beside a 1,500-row table — 50
ticks a run:

| | server, before | server, after | views run / kept, before → after | bytes, before → after | message |
|---|---|---|---|---|---|
| compiled | 0.6 (1.0) | 0.6 (1.0) | 3 / 1 → 0 / 0 | 344 → 174 | patches → values |
| interpreted | 0.6 (1.1) | 0.6 (1.0) | 3 / 1 → 0 / 0 | 344 → 174 | patches → values |

finchdash, a scratch copy on port 3913 refreshed each second (paper
data; Start and Play only, which write page facts), a playing
Simulation's ticks that moved the play-head (41 a run):

| mode | server, before | server, after | views run / kept, before → after | bytes | values turns |
|---|---|---|---|---|---|
| compiled | 5.1 (7.9), 4.9 (14.5) | 4.7 (7.0), 5.6 (7.9) | 13 / 13 → 10 / 11 | 178 | 0 |
| interpreted | 9.9 (10.1), 9.7 (10.0) | 12.3 (35.8), 9.9 (13.9) | 14 / 13 → 11 / 11 | 178 | 0 |

The play bar (`_play_bar`, now a copy whose `secs` is bound) and the
clock inside the scanner card are set where they were run; the tick is
still a render, because the play-head is read structurally where
`_session_over` hands it to the chart grid's widget props
(`grid_charts[…, secs, …]`) and to the symbol card's position strip
(`in_trade = max[0, secs - opened]`, then compared), so `render`,
`_body`, `_session`, `_session_over` and the cards that take it as an
argument run. Those are structural by the rule (widget props, decided
above), not by the play-head's type.

What phase 4 needs (the Lory host drawing the tree in WebGL):

- Open `ws://host/_ws/<path>?drawn=1` (with `p=` from the page's HTML,
  or `t=` from a `session` message, to keep a session; neither starts
  one). Messages arrive as JSON text.
- `{"type":"tree","tree":{…},"head":{…}}` first (the page's body as
  5a's `to_json`; `head` its title and meta), then `{"type":"session",
  "token":…}`; later `{"type":"tree","tree":{…}}` whole after a render
  that changed the page, with `ack` (and `head` when it moved).
- `{"type":"values","ids":{"<id>":value},"ack":n}`: set each into the
  element whose `binds` lists the id (`{child: i, id}`: replace child
  `i` with the node given; `{option: name, id}`: set the option, or
  remove it on `null`). No view ran; the layout may need no pass beyond
  the texts' and options' own sizes.
- Send events as a page does: `{"event": handler, "value": JSON text of
  the args array, "seq": n}`; a control's change `{"event":
  "__field:<bind>", "value": …}` (`__select:` for a select), with
  `"then": handler` when the control's tree lists a `change` event (the
  handler runs after the write, in the same turn, where the page offered
  it; a field sends it only on commit); a table's
  `__sort` and `__page` with its index; `{"event":"__resync"}` when the
  tree it holds no longer fits, which brings the tree whole. Each event's
  `seq` comes back as `ack` on the message that answers it.
- `ack`, `head`, `address`, sign-in and download messages are the page's
  own; the dev panel's `dev` message, with `CROW_DEV`, counts the views.

Left open after phase 3:

- finchdash's playing Simulation tick is still a render (10 views
  compiled): its play-head reaches widget props and a position's
  arithmetic, which are structural. Closed 2026-11-16: a growing int, a
  decimal and a moment bind (above).
- A plain function of the program's that builds nodes takes no session
  state, compiled, so it places no binding; a value it shows is
  structural (only `view`s get `binds`).
- A frame, list or record a view computes has no identity but its text:
  it is keyed under 4 KiB, else unkeyed, and a view handed one runs
  (finchdash's Overview, its session cards).
- Widget props, table cells and markup are structural (decided above).
- A local assigned more than once or in a branch, a parameter set by its
  view or mixed with another, and a view that reaches itself stay
  structural (`assigned`, `binding_params`, `looping_views`).
- An interpreted idle tick costs its render, 12–14 ms on finchdash.
- A value that moves with time alone, as a start and a rate (3.5), is not
  built.
- The test harness's pages always render; a values turn is a served
  page's.

**Phase 4 (2026-11-16).** The drawn renderer (3.2): Lory draws a served
page's own `ui` view in a headset, from the tree, with no HTML pictured.

What was built:

- *Serving.* With `live`, every app serves `/_drawn`, `/_drawn/` and
  `/_drawn/{*rest}` (`live/drawn.rs`, merged in `CrowApp::serve`). It
  serves the files from `CROW_DRAWN`, or from `~/.crow/drawn` when that
  is not set. A path that names no file is the drawn page's
  `index.html`, which reads the page's path from its own address
  (`/_drawn/execution` draws `/execution`). The drawn page is Lory's
  (`packages/drawn`), not Crow's: Crow ships no page of its own.
- *The tree says more* (5a, `to_json`):
  - a click that asks first carries `ask`, what it then sends
    (`layout::ask_args`);
  - an event carries `value`, its arguments as the page sends them;
  - a table's `rows` carry `sends`, `shown`, `sorts` and `steps`
    (`TableOf::sends`, beside `frame_table_parts`). These are what each
    row's click and actions send, each cell's text as the HTML page
    shows it, each sorted header's `__sort` value and the pager's two
    `__page` values;
  - a chart carries `ts`, its x moments as seconds, and `seeks`, what
    each seek band sends.
- *Markup read back.* A compiled page hands on a node it kept in a
  variable, or passed through a parameter, as markup (§ "What is still
  markup"). On finchdash's Execution page that is the whole page body
  and the sidebar's links, so a drawn page saw 9 markup nodes and drew
  them as text. A `markup` node now also carries `read`
  (`tree::read_back`): the markup parsed as the browser reads it
  (`parse_markup`) and read back as the tree's kinds. It reads back:
  - what the vocabulary's builders write: a field, a check or toggle, a
    radio or tabs, a select, a slider, a progress bar, an icon's `svg`;
  - a frame's table, as `frame_table_parts` writes it, as `rows` with
    its sends, actions, sorted headers and pager;
  - any other element as its tag, or a layout kind's `div` as that kind.
    Its `data-crow-*` wiring becomes events, and its class and style
    become the options a theme reads (`color`, `weight`, `size`, `gap`,
    `pad`, `border`, `surface`, `width`, `wrap`, `columns`).

  `html` stays. After this, the compiled Execution tree has the
  interpreted one's kinds, count for count, except one `failed`. It is
  48 KB where it was 27 KB.
- *Lory's drawn page* (`packages/drawn`: `tree.co`, `vr.co`,
  `drawing.co`, `main.co`, `web/`). The host page (`web/drawn.js`) does
  the following:
  - It opens the socket with `drawn=1` and the session's token, which
    it keeps in `sessionStorage` (`crow-session:<path>`).
  - It hands `tree`, `values` and `head` to the program.
  - It sends what the program prints (`@send`) with a `seq`.
  - It follows `@go` (another page: its socket, the address kept) and
    `@back`.
  - It handles `session`, `sign`, `download` and `url`.

  The program builds gui from the tree:
  - `tree.co` reads the tree into records and sets each value by id
    where its binding stands.
  - `drawing.co` draws each kind and maps each gui event to what the
    HTML page sends: `__field:<bind>`, `__select:<bind>`, a click's
    handler and value, a form's submit, `__sort`, `__page`, `__ask`.
  - `vr.co` is the VR theme. It is kept apart from the view, and its
    sizes are dp, so they read as angles. The page's CSS is not read.
  - `main.co` sets the room: the page 1.75 m away, a sidebar on its own
    panel, what is raised (a modal, a drawer, the runtime's question)
    nearer, notices, a bar (‹ Back, title, Again, Centre), and gui's
    drawn keyboard under a focused field.
- *Every kind has a drawn form*:
  - elements by kind or tag;
  - text, controls, charts (their lines, closes and bars);
  - tables, with a window of columns, sort buttons, the chosen row's
    actions and a pager;
  - markup, as its `read`, or its words where nothing was read.

  What it cannot draw shows as a placeholder that names it: a widget, an
  `img`, a raw `input`, `select`, `textarea`, `iframe`, `video`, `audio`,
  `canvas` or `svg` (one that is not an icon), and an unknown kind.
  finchdash's Execution page has none.
- *Cost.* The panels are laid again only when the tree, a value or the
  focus moves, on a press of a control that keeps state, on typing and
  on a scroll. A full lay of the Execution page is about 45 ms in wasm on
  the desk; the other frames are cheap:
  - A hover outlines the control under the ray.
  - A plain press (a button, a link, a table's own buttons, the bar's)
    is found from the boxes already laid.
  - Every other frame puts back the vertices it kept (`lory.keep_words`).

  Each panel is given only the presses made in its own column, because
  gui lets a press anywhere take the focus away. Without that, the bar,
  laid after the page, took a field's focus as soon as the page gave it,
  and nothing could be typed.

Decisions:

- The drawn page is a Lory package served by a path Crow knows.
  `CROW_DRAWN` names its folder. Crow keeps no WebGL of its own, and an
  HTML page and its served tests are byte for byte what they were.
- A compiled page's markup is read back on the server, in Rust, once a
  tree. The drawn page, Crow compiled to wasm, does not parse HTML.
- The wire is unchanged: a press of a drawn control sends what the HTML
  control sends, so a session cannot tell the two apart, and the
  server's offers check both alike.

Measured on the desk, in Lory Studio's made-up headset (headless Chrome,
WebXR emulated, `drawn_stats()` in `web/drawn.js`). Frames are each
`requestAnimationFrame`'s work. Press-to-change runs from the event sent
to the frame after its answer is drawn. Server work is the dev panel's
`render_ms`. Bytes are the message as sent.

| what | pass mark | compiled | interpreted |
|---|---|---|---|
| Execution, idle frame p95 (180 frames) | ≤ 11 ms | 1.6 ms (median 1.1) | 1.4 ms (median 1.0) |
| Execution, frame p95 over six pager presses (282 frames) | ≤ 11 ms | 7.4 ms (worst 67, the frame a tree is laid) | 5.7 ms (worst 58) |
| Execution, press-to-change (six pager presses) | ≤ 100 ms | median 77, worst 83 ms | median 114, worst 115 ms |
| Execution, server work for those answers | — | worst 8 ms | worst 52 ms |
| Execution, an answer's bytes | — | 44.6 KB (a whole tree) | 27.8 KB |
| tests/drawn/app.co, a tick (a fact a `later` moved, 20) | ≤ 5 ms, ≤ 1 KB | 0.6 ms median, 0.8 worst; 63 bytes | see below |
| tests/drawn/app.co, a press whose answer is values | — | median 15 ms, 221 bytes, 0.7 ms | median 13 ms, 221 bytes, 1.1 ms |

The Execution page does not tick by itself, and no finchdash page sent
a message unasked in 8 seconds (telemetry, scanner, simulation,
jobs-finch, out of hours), so ticks were measured on Lory's test app.
Of the Execution page's 16 targets, one is under 48 dp: the sidebar's
"Myna · finchdash" link, at 47 dp. Nothing was measured in a real
headset: the Quest 3 was not worn for this phase.

Tests:

- `packages/drawn` has tests in four files:
  - `tree.co` reads a tree (a table's sends and sorts among it), sets
    values by id, and reads markup's `read`.
  - `drawing.co` draws a small page and checks that each control has
    its box, an unknown kind is named, a notice stands apart, and an
    icon's and a chart's painters draw. It also checks what a link, a
    field's edit and submit, a form's button, a click that asks first,
    a select and a toggle send.
  - `main.co` draws a frame of a small tree.
  - `vr.co` checks that a control's size is over a target's least.
- `tests/drawn/walk.py` serves `tests/drawn/app.co`, interpreted and
  then compiled, and Lory Studio walks `/_drawn/` with
  `tests/drawn/walk.json` in a made-up headset. The walk exercises
  every control and checks the server's answer each time (`drawn_seen`,
  `drawn_sent`):
  - a button, and a confirmed click;
  - a typed field, Enter, and a form's button;
  - a select, a checkbox, a toggle, a radio, tabs and a slider;
  - a table's row, action, pager and sorted header;
  - a modal raised and closed;
  - a link to another page and back, with the same session;
  - the bar's Again.

  Both modes pass. Lory's `test.sh` runs it.
- Crow has three unit tests:
  - `tree::markup_is_read_back_as_the_trees_kinds`: a field, a select,
    a link's icon, and a table's sends, actions, sorts and pager;
  - `the_tree_as_data_says_what_the_view_said`, now with events'
    `value`;
  - `live::drawn::a_path_names_a_file_of_the_folder_or_the_page`.

Left open after phase 4:

- Nothing was measured in the Quest. Studio's frames are desk Chrome's,
  and a Quest's are slower.
- Interpreted, a pager press to its change is 114 ms. The server's
  render is 52 ms of it, so the 100 ms mark is met compiled only.
- Every answer that is not values sends the tree whole: 44.6 KB
  compiled on Execution, with markup's `html` and `read` both. A
  structural change to part of a page is not sent as a part.
- A lay of a long page, about 45 ms on the desk, is one frame of 58–67
  ms when a tree arrives. A frame lays all of a page, even what is
  scrolled away.
- Interpreted, a `later` that schedules another `later` lands its
  chain as one answer to the press that began it. On the test app, 20
  ticks of 100 ms arrived as one 3.5 KB message 2.2 s after the press.
  Compiled, each lands on its own. This mode difference is not from
  phase 4 and was not looked into.
- A compiled Execution tree holds one `failed` node ("error: loading")
  in the sidebar that the interpreted one does not.
- Markup read back is as good as the markup. An element whose wiring or
  look is not the vocabulary's (a program's own HTML) is drawn by its
  tag, and its CSS classes other than `card`, `up` and `down` are not
  read.
- In the side panel, a row wider than the panel is cut off ("Close the
  s…"), and one target is 47 dp.
- While a field has the focus, the drawn keyboard stands between the
  hand and the page's lower controls, and a ray aimed at them hits the
  keyboard. Enter in a single-line field lets the focus go, which puts
  the keyboard away. A multi-line field has no such key yet.
- Images, video, iframes, widgets and a raw `svg` are placeholders.
  Kite's widgets (finchdash's chart grid) are not drawn.

**Phase 4, after the first headset run (2026-11-16).** The first run in
the user's Quest 3, on finchdash's Execution page, measured three faults:

- a press took 314 ms (median) to its change, and each answer was the
  whole tree, 44.4 KB, laid again whole;
- with the drawn keyboard up a frame took 16 ms (median) where it took
  5.8 ms without it;
- on entering, the page stood low, to the right and tilted.

What was built:

- *Answers as changes.* A drawn client that opens with `changes=1` is
  sent the tree whole only on connecting, on a route change and on a
  resync. Every other answer that is not plain values is
  `{"type": "changes", "changes": [...]}` (`tree::drawn_changes`), each
  change at a path through the tree's `children` (and a markup node's
  `read`): `{at, node}` replaces a node, `{at, own}` sets an element's
  own fields with its children kept, `{at, splice: [from, drop], put}`
  splices children. The server keeps the tree it last sent the client,
  and sends the tree whole when the changes are not under three
  quarters of it. `tree::apply_drawn_changes` is the client's half, and
  a randomized test checks that the changes make the next tree. The
  tree sent a drawn client leaves out `html` beside `read` and a frame
  table's row markup (`drawn_trim`). A client without `changes=1` is
  sent what it was; an HTML page is sent what it was, byte for byte.
- *Laid by the part.* The drawn page lays each panel (main, side, the
  raised layer, the bar) by itself, and only when its part of the tree,
  its focus or a press in it changed it. Within a panel, every element,
  table and chart is a part gui keeps (`gui.kept_as[node, name, sig]`,
  the sig the node's mark, its width and path): laid once, its vertices,
  boxes, Tab order and hover kept, and put back, moved where it now
  stands, while its mark holds and the reader does nothing in it. The
  tree's marks (`tree.Marks`) are set where a change or value lands. A
  pager's press lays the table alone.
- *The keyboard.* The drawn keyboard's keys are laid once, by caps, and
  the key under the ray is drawn over them; before, gui laid it with the
  page, four times a frame.
- *The buffer.* Only the run of vertices that changed is uploaded
  (`bufferSubData`); the whole buffer only when it outgrows its room.
- *Typing.* A key is not a round trip. A field of a form that submits
  sends its words when the form does or the focus leaves it; any other
  field 300 ms after its last key, on blur, or before any other send.
- *Standing.* The page is stood before the head on the first 30 frames
  that hold a real pose (`xr.posed`: a view away from the room's
  origin), and again after a gap in the frames over 750 ms; a headset's
  first frames may hold no pose.
- *Compiled field moves.* `p.c = f[p.c]` hands the field to the call and
  takes it back, where it copied it (`emit_field_through_call`); gui's
  canvas is such a field, so every kept part had copied the canvas.
- *Stats.* `drawn_stats()` adds the time between frames (`interval_*`),
  each message's parse time (`parse_*`), the work of the frame that took
  a tree, changes or values in (`layout_tree_*`, `layout_changes_*`,
  `layout_values_*`), and the answers by kind with each kind's worst
  bytes.
- *Tests.* Lory's `tests/drawn/walk.py`, compiled, also walks
  `walk_cost.json` at a 4x CPU slowdown: typed keys send nothing and
  Enter sends the field then the form; the frames with the keyboard up
  are under 8 ms at p95; the pager's answers are changes under 4 KB,
  drawn under 80 ms at p95.

Measured on the desk, in Studio's made-up headset, finchdash's compiled
Execution page on a scratch copy (Chrome as it is, and slowed with CDP's
CPU throttle):

| | before, 1x | 1x | 4x | 6x |
|---|---|---|---|---|
| a pager's answer | 24.2 KB tree | 3.2 KB changes | same | same |
| press to change, median | 92 ms | 33 ms | 91 ms | 139 ms |
| the frame that lays a pager's change | the tree laid, 23 ms | 12 ms | 45 ms | 69 ms |
| idle frame, median / p95 | 1.5 / 2.0 ms | 1.1 / 1.7 ms | 2.6 / 3.4 ms | 4.2 / 6.2 ms |
| keyboard up, median / p95 | 64 / 68 ms | 1.0 / 1.5 ms | 3.2 / 4.7 ms | 4.3 / 6.7 ms |
| a typed field's answer | one per key | 740 B, once | | |

The brief's mark, p95 under 11 ms at 6x with the keyboard up, is met
(6.7 ms). The old crow on 3940 sends Execution's tree as 44.6 KB, laid
in 75 ms at 1x; the desk's 24.2 KB is the trimmed tree. There is no
before at 4x or 6x: the old build was overwritten before they were
measured.

Left open:

- Ids are paths, so a splice shifts its later siblings' paths and lays
  them again, and a typed field's splice still lays its panel (34 ms at
  1x, 200 ms at 6x). Keyed ids would keep them.
- Applying a change copies the siblings on its way (`_changed_at`,
  `_set_at`), and a view built from the tree is built whole on each
  version.
- Working out the targets (`targets_said`) and reading at a path
  (`at_path`) cost a few ms after each change.
- The caret's blink lays its part again: 7 ms at 1x, 29 ms at 6x.
- Not yet worn: these are the desk's numbers, not the Quest's.

**Phase 4, after the second headset run (2026-11-16).** The second run
in the user's Quest 3 (finchdash's Execution page, the compiled server,
`drawn_stats()`): an idle frame's work 4.3 ms median, 5.1 p95, at 72 a
second (13.9 ms) and steady; 30 presses, paging and typing, each answered
by changes (the worst 3105 B); a press to its change 90.8 ms median, 142
p95, 191 worst (the mark is 100); the frame that laid a change 62.6 ms
median, 182 worst; frames dropped at each change (the worst gap 41.8 ms).
The user: "placement is at correct distance, but i would like to be able
to move it using the controller as i want"; on a search field two
keyboards came up, the drawn one and the Quest's, and typing in the
Quest's showed nothing; the drawn keyboard typed without lag.

What was built:

- *Moving panels.* The page (its main panel, side and bar as one), a
  raised panel and the keyboard are taken by a controller's grip held
  while its ray is on them, or by a press (the trigger, or a hand's
  pinch) on a handle: Move, on the bar, at the top of a raised panel and
  under the keyboard. What is held follows the hand's ray as Lory's
  window does (`window.moved`: the offset of its way from the ray's,
  taken when it is grabbed), as far from the head as it was, the stick
  taking it from 0.3 to 5 m, upright and turned to face the reader.
  Letting go (the grip, or the trigger that took it) leaves it. The
  events a grab takes are given no control, so a grab never presses, and
  the hand that holds a panel scrolls nothing. The raised panel and the
  keyboard are kept where they were left in the page's upright frame
  (`Place`), so they move with the page and the keyboard comes back where
  it was left; Centre forgets them.
- *One keyboard.* Meta's browser raises its own keyboard when a page
  focuses a text field in an immersive session (Browser 26.1 and on,
  `isSystemKeyboardSupported`), and gives the page only the field's whole
  value, a new edit each time it comes up; the loader cleared the value
  on every input, so its typing showed nothing. gui focused the loader's
  hidden field whenever one of its fields took the focus, also in a
  headset where it draws its own keys. Now gui asks for no phone keyboard
  in a headset (`xr.in_headset`), and the loader focuses nothing while a
  session runs, and lets go of any focus as one starts. Outside a session
  a desk's keys type as before (a key with no field focused is the
  program's `text`).
- *Keys, not paths.* Each node of the tree held has a key (`D.key`),
  given as it is read and kept while changes land around it, given back
  when it goes. Marks, the ids of controls (`p:41`, `w:41`) and gui's
  kept parts are by key, so a splice lays the change, not its later
  siblings. A change is applied by reference, the siblings on its way not
  copied; `at_path` and `path_to` walk the tree without copying it. An
  element whose own fields change keeps its children's kept drawings.
- *By the panel.* Each panel's controls are built only when its part of
  the tree changed (`PanelSigs`), and kept by name (`drawn.view/<k>`) so
  a frame lays one without reading the others. The names of what can be
  pressed are read once a version, in one walk.
- *Spread over frames.* The frame that hears a change takes it in, the
  next builds the panels it touched, the one after lays them; the first
  tree is built and laid at once. What can be pressed is worked out on a
  frame that lays nothing (or after three). The program prints `@shown
  n` when its panels show the first n trees, values and changes it took,
  and the host measures by it: a press's time to its change ends at the
  frame that shows it, and `layout_*` is the worst frame from a message's
  coming to its showing. `wait_*` is the server's part, `shown_frames_*`
  the frames from the answer's coming to its showing.
- *The caret.* In a headset gui's caret is steady: a blink laid the
  field's part again.
- *The atlas.* Quotes, arrows and marks pages show are in the atlas from
  the start: a glyph the atlas lacked made it again, and every panel was
  laid again with it (a search's answer laid the page three frames
  running, 117, 60 and 41 ms at 4x).
- *Hooks.* `drawn_press(name)` presses what `lory_targets()` names (or
  its id) as a trigger does: the pointer on it, a ray's down one frame
  and its up the next, through the same events. `drawn_type(words)`
  presses the drawn keyboard's keys. A head looking almost straight up or
  down keeps the page's turn, so a session started with the headset lying
  on its face or back stands the page level.
- *Tests.* Lory's `tests/drawn/walk.json` takes the page by the grip and
  moves it (the stick changing its distance), checks that the grab sent
  nothing and that a control still presses where the page went, moves the
  page by its handle and the keyboard by its handle, checks that the
  keyboard comes back where it was left and that Centre puts both back,
  and that no field of the page has the focus in a session.
  `walk_cost.json` at 4x bounds the frames while two keys are typed
  (under 20 ms), the frames with the keyboard up (95 in 100 under 8 ms),
  the frames from a pager's answer to its showing (under 16 ms, shown by
  the third frame). Studio's drag takes `"grip": true`.

Measured on the desk, in Studio's made-up headset, finchdash's compiled
Execution page on a scratch copy (before: this phase's start):

| | before 1x / 4x / 6x | after 1x | 4x | 6x |
|---|---|---|---|---|
| worst frame of a pager's change | 12 / 42 / 65 ms | 5.6 ms | 25 ms | 39 ms |
| its frames (taken in, built, laid), 4x | one frame | | 8 / 10 / 22 ms | |
| a typed field's answer, worst frame | 34 / 121 / 178 ms | 7.7 ms | 39 ms | 57 ms |
| a frame that types a key | | 5 ms | 20–22 ms | 34–37 ms |
| keyboard up, p95 / worst (the caret steady) | 6.7 ms p95 at 6x, a blink 29 ms | 1.2 / 1.5 ms | 5.2 / 6.0 ms | 7.5 / 9.4 ms |
| idle frame, median | | 0.9 ms | 3.6 ms | 5.6 ms |
| press to change shown, median / p95 | | 68 / 91 ms | 121 / 126 ms | 160 / 168 ms |
| of it the server's, median | | 11.5 ms | 32 ms | 49 ms |

A press's time to its change in Studio is paced by the walk (its `until`
pumps a frame and then waits), so it says little of a headset's: every
answer was shown by the third frame after it came, which at 72 a second
is 42 ms after the answer.

What was reached, against the brief's marks at 6x: the caret makes no long
frame (met). A pager's change is no longer one frame but three, the worst
39 ms (the mark is 11, never above 22: not met; 25 ms at 4x). A key typed
lays its panel, 34–37 ms (the mark is 11: not met). Press to change p95
under 100 ms cannot be judged on the desk.

Left open:

- Laying the part that changed is the cost now: gui's laying of the new
  table (a fifth of a change's time at 4x), and the canvas's vertices
  sent to WebGL as text after it (`lory.end`, about a quarter of every
  frame's time). A change cannot yet be laid over two frames.
- A key typed lays the whole panel its field is in, putting back every
  kept part (about 16 ms of work at 4x on Execution).
- The tree held is copied whole each time it is read or kept
  (`recall`/`remember`), about 4 ms at 4x in the frame that hears a
  change.
- The first showing of new content (a search's results) lays it all:
  39 ms at 4x.
- Not yet worn: grabbing, the one keyboard in Meta's browser and the
  hooks over a cable are the desk's, not the Quest's.

**Three faults a real app found, 2026-11-16.** Found by restructuring
finchdash's Simulation; each reproduced by a small test first, fixed in
both modes.

- *A derivation brought up to date inside a view leaked its reads into
  the view.* `view_read_derive` (crow_web/src/live/views.rs) recorded every
  derivation read while a view ran, including those read inside another
  derivation's own reading: its inputs brought up to date first (compiled
  `__derive_d`'s `let _ = __derive_x(state)`, the interpreter's
  `served_read` loop) and its body computed. So a view that showed `words`
  (a derivation over `secs`) bound counted `secs` among its structural
  reads, ran at every move of `secs` and was never a values turn; a view
  over `band` (`secs gt 25`) ran even when `band` held. Now a derivation's
  own reading runs under `CrowLiveState::deriving` (`compute_now` around
  every body, the compiled `__derive_*` and the interpreter's
  `served_read` around the inputs), and a read inside it is no view's: a
  view depends on the derivation it read by its version alone. Tests:
  served `47_a_view_over_a_derivation_of_a_derivation` (a tick a values
  turn of one place; a tick that moves `secs` and not `band` keeps
  `band`'s view); `live::views::tests`
  `a_derivation_read_in_a_view_brings_none_of_its_reads_into_it`.
- *The compiled page froze a clock read only in bindings.* Not the clock:
  a compiled view's parameter that takes a node was held as its markup
  (`body: String`, the caller writing `.to_html()`), where the
  interpreter holds the node. finchdash's pages hand their body to Myna's
  `workbench.screen[section, where, actions, body]`, so the page's root
  held the body as markup: its bindings and kept views were inside text.
  A values turn found the session's bindings moved, set them in the kept
  views' own nodes, and set nothing on the page (no binding was placed in
  it), so the play-head stood still; a render ran the views and their
  nodes were thrown into markup again. Now a view's parameter of a node
  type that its body only places — an element's child, its answer, a
  local it may give another node (`shown = body`, then `shown =
  access.no_access[]`), or a node parameter of a view it hands it to (a
  greatest fixpoint, `Emitter::node_params`, src/emit_rs/stmts.rs) — is a
  `tree::Node`, and its callers hand the node (`user_call`, exprs.rs); the
  analysis is the one `node_locals` makes now (`node_holders`). A view
  held as a function value keeps its markup parameters. Such a view is
  not keyed, as the interpreter keys no view handed a node (it had been
  keyed by the markup, under 4 KiB). Tests: served
  `49_a_bound_clock_inside_a_body_handed_on` (a body handed to a frame
  that holds it in a local and hands it on: each tick a values turn of 2
  places, the message held); parity `535_a_view_handed_a_node_places_it`.
- *Under `CROW_VIEW_CACHE=check`, compiled, a date field written with
  `__field:` did not move the kept node.* The same cause: the field sat
  in the markup the root's kept node held, which no rebinding reaches,
  so the kept node showed `value=""` where the render, every view run,
  showed `value="2026-11-06"`. The oracle was right and the kept node
  wrong; nothing about date fields. With the body a node, the field is a
  bound control on the page and is set as any is. Test: served
  `48_a_date_field_written_is_set` (date, time, number and text fields in
  a form handed to a frame; each write a values turn; with the cache on
  and checked). The compiled page had failed it with the cache on too:
  the written value never reached the page's input.

Measured on finchdash, a scratch copy (paper data; Start and Play, and
for the busy run the Date field, the scrubber and Seek, which write page
facts only), `refresh = 1`, 30 s a run, the dev panel's words; before is
the same tree with the two fixes turned off. "Clock-only" is a playing
tick that moved nothing but the play-head (Start, Play at the window's
start); "scan" a playing tick at 9:10 on 2026-11-06 that moved the
scanner's rows. The app's workaround for the frozen clock (`_play_clock`
reads the play-head structurally) is kept in one copy and undone in the
other (its view shows `store.clock[clock_secs]` as a bound text). Medians, 90th percentile in brackets; the
machine was loaded and its load moved between runs, so the times move by
up to 3x between runs of one build: the views, bytes and kinds of turn
are what to read.

| tick | copy | mode | before | after |
|---|---|---|---|---|
| clock-only | workaround | compiled | render: 6 run / 11 kept, 7.15 (20.5) ms, 178 B | render: 7 / 11, 1.95 (2.3) ms, 178 B |
| clock-only | workaround | interpreted | render: 7 / 11, 10.5 (16.3) ms, 178 B | render: 7 / 11, 9.3 (9.6) ms, 178 B |
| clock-only | undone | compiled | nothing sent: the clock stood still (99 turns, 0 B) | values turn: 0 / 0, 1.3 (1.6) ms, 111 B, 2 places |
| clock-only | undone | interpreted | values turn: 0 / 0, 12.4 (20.7) ms, 111 B | values turn: 0 / 0, 9.6 (9.7) ms, 111 B |
| scan | workaround | compiled | render: 14 / 10.5, 7.6 (19.5) ms, 11,063 B | render: 11.5 / 9.5, 4.5 (5.3) ms, 11,063 B |
| scan | workaround | interpreted | render: 18 / 14, 50.9 (62.7) ms, 11,874 B | render: 16 / 10, 45.3 (48.3) ms, 11,874 B |
| scan | undone | compiled | render: 13 / 10, 5.35 (8.0) ms, 11,063 B | render: 10.5 / 9, 4.6 (5.2) ms, 11,063 B |
| scan | undone | interpreted | render: 16 / 13, 57.6 (69.8) ms, 11,874 B | render: 14 / 9, 47.0 (48.8) ms, 11,874 B |

With `CROW_VIEW_CACHE=check`, before, the compiled page stopped at the
Date field's write (fault 3, as in finchdash's logs); after, the
workaround undone, both modes ran both scenarios with no stop, the
clock-only ticks values turns in each. The compiled page sends a turn
of nothing between ticks (a background value landing), as before.

The workaround can go from finchdash (app/simulation.co): `_play_clock`
shows `store.clock[clock_secs]` as a bound text, its comment and the
cards' note changed to say so. A scan-moving tick still renders: the
scanner's rows, the chart widgets' props and the position's arithmetic
are structural by the rule (phase 3).

Left open:

- A non-page compiled program whose view reads a fact does not build
  (`cannot find value 'day'`, seen writing parity 535; not looked into,
  nor whether it predates these fixes).

**Phase 4, widgets and charts drawn (2026-11-16).** A widget's module
runs only in the HTML page; a headset could show a placeholder. Now a
widget states its drawn counterpart, and Lory draws it, no
functionality lost where a counterpart exists.

- *The mechanism.* Beside `widgets/<name>.js` an app may put
  `widgets/<name>.drawn.json`, what the widget is drawn as: `{"draws":
  "kite.charts", "bars": "/finch/bars", "candles": "/finch/candles"}`,
  `{"draws": "lory.table"}`, or `{"draws": "lory.text", "buttons":
  [["Save", "save"], …]}`. The widget route serves it
  (`/_widget<page>?m=<name>&drawn=1`, `application/json`, 404 where there
  is none); the drawn program asks for it once a module, through
  `lory.request` (no change to the tree's JSON: a widget node already
  carries its module, props and events). Until it comes, and where there
  is none, the widget is a placeholder that names it and says why. Lory:
  packages/drawn/widgets.co.
- *Charts* (`kite.charts`): the props' `spec` (finchdash's
  app/charts.co format: candles, mini, bars, line, with marks, levels,
  bands, volume, VWAP, play-head) read by Kite's wall
  (`wall.grid_read`, ported from Myna's room), laid as a box of the
  wall's height in the panel (`gui.area`), drawn after the panel in the
  panel's coordinates, clipped to its window and cut at its edges. The
  pointer reads a bar (Kite's readout and crosshair); a chart of candles
  is panned by a drag or the stick across and zoomed by the stick along
  or a wheel (`wall.viewed`; the panel does not scroll under it); a press
  that did not drag on a bar or a small chart that offers a pick sends
  the widget's `pick` with the words its module sends (`"<chart id>
  <bar>"`). Bars and candles are fetched from the routes the descriptor
  names. Each chart is a target of its own (`chart <title>`).
- *Tables* (`lory.table`): the spec (columns, data, key, actions,
  focus) drawn by the drawing's own tables: sorted and paged here (25 to
  a page; nothing sent), the row its `focus` names chosen, a row's press
  sending `pick` and its actions their lower-cased events with the row's
  key, as finchdash's grid module does.
- *Words* (`lory.text`): a field of many lines (`change` on each edit)
  and the descriptor's buttons, each sending the words as edited.
- *Pictures.* An `img` is loaded as a texture (`lory.load_image`) and
  drawn at the size its attributes or its loading give; an `svg` the
  page writes as markup (a kernel chart) is read with a `data:` address
  of its markup (crow_web view/tree.rs), and drawn the same way.
- *Live updates.* A change whose node is a chart widget of the same
  module and height is taken quietly: the tree holds the new props but
  nothing over it is marked made anew (`tree.with_changes_marked`), so a
  tick that moves only the play-head builds and lays nothing; the other
  changes of the message are taken as before.
- *Hidden targets.* A control laid but not wholly in its panel's window
  is named in `@targets` with `"hidden": true` (a table's rows taller than
  the window by the part shown); the host's `drawn_reveal(name)` scrolls
  its panel to it, `drawn_press` reveals before it presses, and Lory
  Studio's walks do the same and size only what is shown.

Measured (Lory Studio, desk, the drawn page's own frame): the test app's
six charts shown, pointer on the candles, 1.9 / 8.6 / 13.3 ms median
(p95 2.1 / 9.5 / 14.9) at 1x / 4x / 6x, 106 draws a frame; a small
chart's pick answered and shown 3 frames on, worst frame 3.7 / 16.3 /
24.5 ms; the play-head's ticks, worst 4.6 / 17.1 / 24 ms. finchdash's
Reports page with its Daily P&L chart shown: 0.8 / 3.4 / 5.5 ms median,
p95 1.1 / 4.4 / 7.0. A pager's change at 6x: worst frame 13.2 ms; a
typed key's: 10.4 ms. The charts' cost is Lory's 3D drawing (`space.draw`
38%, `lory.end` 21%, memory taken and given back 43% of the time).

Left open: six charts of candles cost 13 ms a frame at 6x; caching a
chart's draw calls while nothing moved would cut it, but space's state
tracking and its second program make a replay unsafe as it stands. The
wall's lettering is 10–15 dp, under the 14 dp target. A chart of
candles' view (dragged, zoomed) is the page's own, not sent. The editor
widget's drawn form is words only (no highlighting, no run output).

**Phase 4, the sweep of a real app (2026-11-16).** finchdash and the
Myna pages it joins, 21 pages and states, each drawn by Lory
(`/_drawn<path>`) and checked against its HTML page in Lory Studio's
made-up headset by a script that can be run again: Lory's
`tests/drawn/sweep.py` with `tests/drawn/sweep_finchdash.json` (a fast
mode, about 110 s; `--function`, about 220 s). The drawn page writes down
what a frame drew when asked (`drawn_audit()`: every text and box with
its place, clip and colour, each panel's window, the targets and the
tree's words); the sweep checks content (every node's words drawn, none a
placeholder), layout (nothing cut, past an edge, over words or crossing a
box), legibility (words ≥ 14 dp, targets ≥ 48 dp, from where the reader
stands) and function (each button pressed dry, `drawn_dry`, against the
HTML click; tabs, radios, selects by keys, fields on the drawn keyboard,
disclosures, tables paged, sorted, windowed and pressed, a link and
Back). Order buttons (BUY, SELL, the ladder, Cancel All) are drawn and
targets and never pressed.

| page | before: content / layout / legibility | function, first run | after: all four |
|---|---|---|---|
| Execution | 1 / 7 / 1 | ok | ok |
| Reports, Overview | 1 / 32 / 1 (34 chart words once charts were audited) | 6 | ok |
| Reports, Breakdowns | 1 / 32 / 1 | 6 | ok |
| Reports, Calendar | ok / 34 / 1 | 6 | ok |
| Reports, Journal | ok / 32 / 1 | 6 | ok |
| Reports, Trades | 1 / 32 / 1 | 6 | ok |
| Simulation | ok / 13 / 2 | 2 | ok |
| Simulation, playing (not prepared before) | 1 / 34 / 2 | 3 | ok |
| VWAP Scanner | 1 / 11 / 1 | ok | ok |
| Budgets | ok / 3 / 1 | ok | ok |
| Telemetry | 3 / 8 / 2 | ok | ok |
| Jobs (finch) | 2 / 12 / 1 | ok | ok |
| Myna Projects | 1 / 28 / ok | ok | ok |
| Myna Catalog | 1 / 2 / 1 | ok | ok |
| Myna Query | ok / 7 / 1 | 1 | ok |
| Myna Code | ok / 38 / ok | 2 | ok |
| Myna Dashboards | 1 / 121 / ok | ok | ok |
| Myna Experiments | ok / 22 / ok | ok | ok |
| Myna Models | ok / 8 / ok | ok | ok |
| Myna Ask | ok / ok / 1 | ok | ok |
| Myna Jobs | ok / ok / 1 | ok | ok |

(509 faults before, none after: the fast sweep and `--function` both end with no fault.)
What was fixed, in the renderer and generally:

- *Rows wrap.* `gui.row[…, wrap = true]` flows its controls onto lines
  as wide as the room; every row the drawn page makes wraps. The long dark
  bar over Reports' filters was a quad crossing a panel's column edge,
  drawn with the next panel's matrix: each panel's controls are clipped
  to its own column, and the shader picks the panel from a vertex's clip.
- *Words.* Text breaks at `\n` and inside a word wider than the room (a
  path); a field's hint too long for it ends in an ellipsis.
- *Targets.* A theme's least height of what is pressed (`least`, 34 CSS
  px, 49 dp) for buttons, fields, selects, toggles, checkboxes, radio
  options, entries and sliders; the side panel, farther at its edge,
  1.17 times it.
- *Charts.* Kite's wall draws no word under a page's least
  (`wall.letters_least`, 14 dp drawn), its labels kept inside the chart;
  charts laid for another width are laid again before they are scrolled
  to, so the page is as tall as they are.
- *Function.* A control inside a scrolled column is a hidden target until
  the column shows it, and `drawn_reveal` scrolls the column
  (`gui.scrolled_to`); a shut disclosure's insides are not its words.
- *Cost.* Lory's `space.draw` copied the canvas's list of GL steps for
  every shape (a field read copies); it now adds its own. Six charts, the
  pointer on one (`walk_charts_cost.json`), p95 a frame: 1x 2.1 → 1.5 ms,
  4x 9.0 → 6.4, 6x 13.5 → 9.3 (median 12.2 → 8.1). What is left is mostly
  `lory.end` writing the frame's steps as JSON.
- *Crow.* Two compiled-only faults (parity 536, 537) fixed in the
  emitter, Lory's workarounds taken out: `x in xs` of a list whose items
  can fail moved `x`; `a and b and c` with a `b` that can fail wrote its
  last step as a plain truth.

Left: 6x is 9.3 ms, not 8 (the next step is the frame's steps sent other
than as JSON); a table's columns behind its column window and a few
words of the HTML page the drawn page shows differently (an account's
label, a number's format) are noted, not faults; an unbound name in a
package's function is reported as an internal error of Crow's
("the lowered program disagrees with itself") rather than as the
program's.

### Phase 5, the mobile theme (2026-11-17)

The HTML renderer's mobile theme: a phone gets it with no change to an
app's views. It is the kernel stylesheet's own tokens with a phone's
values, plus a few structural rules.

- *Tokens.* The kernel stylesheet (`crow_runtime::layout::DEFAULT_CSS`)
  now takes its sizes from tokens on `:root`, set to the values it had:
  `--crow-font-size` 15px, `--crow-line-height`, `--crow-page-max`,
  `--crow-page-pad`, `--crow-heading-1/-2/-3`, `--crow-control-font`
  (buttons, fields, selects, tabs, checks, a dropdown's summary),
  `--crow-control-pad`, `--crow-field-pad`, `--crow-label-font`,
  `--crow-table-font`, `--crow-column-head`, `--crow-head-pad`,
  `--crow-cell-pad`, `--crow-card-label`, `--crow-action-font/-pad`,
  `--crow-tab-pad`, `--crow-link-pad`, and `--crow-target` (`auto`: the
  least height of every control and a button's least width). The density
  and wide rules stay as they were.
- *The phone's values* (`MOBILE_RULES`, `mobile_css()` beside the kernel
  stylesheet; the page shell carries it after the runtime's own rules and
  before an app's `css/app.css`): 16px body copy, controls and labels
  (16px also stops iOS zooming into a field), headings 1.75rem / 13px /
  1.125rem, column heads and card labels 12.5px (captions),
  `--crow-target` 44px, cells padded to a 44px row, the page padded
  12/16/32px or more by the safe area (`viewport-fit=cover` is now in the
  shell's viewport meta). The phone's own tokens: `--crow-grid-min`
  280px, the type steps `--crow-type-xs/-s/-l/-xl` (13/16/22/26px for
  `size =` 12/14/20/24, the inline values matched by attribute), and
  `--crow-surface-name: mobile`, which a check reads.
- *Structure.* Rows wrap; a vocabulary child is never wider than its box;
  grids become as many columns of at least 280px as fit (one on a phone
  upright; a grid's own `below`/`columns_then` still wins); a table
  scrolls in its own box with its first column held at the left; `pre`
  scrolls; media fit; tabs scroll; nav links, a stack's or row's links
  and dropdown entries are 44px boxes; a clicked element is at least 44px
  tall; modals, drawers, toasts, the runtime's confirm dialog, notice and
  offline banner sit inside the safe area; compact density is set back to
  the phone's sizes.
- *When.* `(pointer:coarse)` with a side under 600px: a phone either way
  up. A narrow desktop window keeps the desktop look, so the layout
  engine and its 390px conformance fixture still describe what Chrome
  draws there. `<html data-surface="mobile">` forces the theme,
  `data-surface="desktop"` turns it off; nothing in Crow sets either yet
  (`serve[]` has no surface argument). Every rule is scoped with
  `:where`, so it adds no specificity: an app's stylesheet still wins
  where it speaks.

The check, `tests/mobile/check.py` (Chrome over DevTools, no
dependencies but `websockets`), opens each page at 375x812 with touch
emulated and measures sideways overflow, elements wider than the screen
that no box of theirs scrolls, controls under 44x44 (words of a sentence
exempt), text under 16px that no caption rule allows (`th`, `h2`,
`.card h3`, `size = 'xs'`, `small`; code at 14px), and every layer
forced open plus the runtime's own. With `--record`/`--compare` it holds
the computed styles of a sample of elements at 1280x800 against a
recording, property by property.

| pages | before: overflowing / small targets / small text | after |
|---|---|---|
| the repo's 17 (conformance, ui primitives, plain and compact+wide, 10 served apps, 2 examples) | 2 (up to 321px) / 100 of 106 / 205 of 219 | 0 / 0 / 0 |
| finchdash in Myna's shell, 20 pages | 12 (up to 170px) / 1195 of 1295 / 4481 of 4943 | 0 / 0 / 215 |

Desktop at 1280x800: 387 elements of the repo's pages and 1028 of
finchdash's, every computed property, identical before and after.

The 215 small texts left are Myna's stylesheet setting pixel sizes:
`.tree .treefile` and `.tabs .tab button:first-child` 12px (186 on the
Code page), `.themeflip` 12px, `.qform .crow-field label` 12px,
`a.button` 0.92rem, `.src pre` 13px. An app keeps the theme by sizing
with the tokens (`font-size: var(--crow-control-font)`) or the
vocabulary's `size =`, not pixels. Myna's open sidebar on a phone is a
wrapping bar a screen tall, its own design (closed, it is one button).
Left: the exported documents (`crow doc`, `markdown::document`) do not
carry the mobile theme; the drawn renderer's mobile surfaces are not
part of this.

## 8. What this does not cover

- Moving `ui` out of the language (decided against).
- Lory drawing pages on the ordinary web.
- Native headset apps.
