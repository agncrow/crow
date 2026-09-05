# Charts and Styling

Two things every real dashboard needs, and neither costs you a line of
CSS or JavaScript: a chart over your data, and a page that looks right.

## chart: a view over a frame

`chart` is an element like `table` — valid inside views, it takes a
frame and named arguments naming the columns to draw, and renders as SVG
on the server:

```crow,norun
use web, ui, db, frame

conn = db.connect['sqlite://bars.db']

view render[]
    bars = query[conn, 'SELECT ts, o, hi, lo, c, vol FROM bars ORDER BY ts']
    return div[
        h1['AAPL — 1 minute'],
        chart[bars, candles = ['o', 'hi', 'lo', 'c'], bars_y = 'vol',
              tz = 'America/New_York', height = 240],
        class = 'card'
    ]

page['/', render]
serve[3000]
```

The named arguments are the whole vocabulary:

| Argument | Default | Draws |
|---|---|---|
| `x` | `'ts'` | the column of moments on the x axis |
| `candles` | `[]` | open, high, low, close columns as candlesticks |
| `lines` | `[]` | columns as line series (`lines = ['vwap', 'ema']`) |
| `bars_y` | `''` | a column as a volume pane |
| `marker` | `0` | a moment marked with a vertical play-head |
| `tz` | `'UTC'` | the timezone the axis ticks are labeled in |
| `height` | `320` | height in pixels |

Three properties follow from "a chart is a view over a frame":

- **It re-renders like everything else.** Query at render time, or read
  a reactive variable, and the chart tracks the data — `serve[3000,
  refresh = 5]` from the live-loop section keeps it moving.
- **Empty is not an error.** A frame with no rows renders a placeholder,
  so a chart before the first tick doesn't blank the page.
- **Too much is an error.** A frame over 1000 points refuses with an
  error that names the fix — aggregate to what a screen can show, with a
  coarser `GROUP BY` in the query or `head[frame, n]` — because a chart
  that repaints ten thousand points every refresh would freeze the live
  loop the last section warned about. It never downsamples silently.

The colors come from the style tokens below, so a chart is theme-correct
in light and dark mode without any color plumbing.

## Layout: compose boxes, don't write CSS

A page is composed from five containers — `stack`, `row`, `grid`,
`overlay`, `scroll` — and a fixed set of layout attributes that any element
accepts: spacing from a scale (`gap = 'm'`, `pad = 'l'`), sizes
(`width = '240px'`, `'50%'`, `'fill'`), type (`size = 'xl'`, `weight =
'bold'`), surfaces (`surface = 'card'`, `radius`, `shadow`), colours as
tokens (`color = 'up'`), and responsive rules at named widths
(`below = 'phone', columns_then = 1`). There is no CSS to write, and a
wrong value is a compile error that names the scale.

Because the vocabulary is closed, Crow ships a layout engine — written in
Crow — that computes where everything goes. Tests read it through
`box[]`, `style[]`, and `fits[]`:

```crow
use web, ui

min_qty = 0

fn reset[]
    min_qty = 0

view stat_card[label, value]
    return stack[h3[label], p['`value`', size = 'xl', weight = 'bold'], surface = 'card']

view render[]
    return stack[
        h1['Trading Dashboard'],
        grid[stat_card['P&L', '1,116.10'], stat_card['Trades', '4'],
             columns = 2, below = 'phone', columns_then = 1],
        row[field['Min qty', min_qty], button['Reset', click = reset], gap = 'm'],
        overlay[scroll[p['the tape'], height = '120px'], span['LIVE', color = 'up', anchor = 'top_right']],
        gap = 'l', pad = 'l'
    ]

test layout[]
    s = app[render, width = 1280]
    cards = box[s, 'grid .card']
    assert[len[cards] eq 2]
    assert[cards[0].y eq cards[1].y]                --- side by side on a desktop
    controls = box[s, 'row'][0]
    assert[controls.y gt cards[0].y + cards[0].height]
    assert[fits[s, 1280]]
    assert[style[s, '.card'][0].background eq '--crow-surface']
    assert[style[s, '.card p'][0].size eq 'xl']
    phone = app[render, width = 390]
    stacked = box[phone, 'grid .card']
    assert[stacked[1].y gte stacked[0].y + stacked[0].height]   --- one under the other
    assert[stacked[0].width eq stacked[1].width]
    assert[fits[phone, 390]]

page['/', render]
log['dashboard defined']
```

```text
dashboard defined
```

Read the test block as claims about the page at two widths: two cards
side by side at 1280 pixels and stacked at 390, the controls under them,
the card on the surface colour, the big number at the `xl` step, and
nothing overflowing at either width. Every claim is checked by the same
engine that determined the page's layout, and the browser lays out the
emitted CSS by the same rules — see `docs/ui-engine-design.md` for how
that agreement is kept.

## Run the layout example yourself

The full version of that dashboard — three cards, an alert badge in an
`overlay`, a control `row` that turns into a stack on phones, a
scrolling tape — lives in the repository as
[`examples/layout_dashboard.crow`](https://github.com/vsriram/crow/blob/main/examples/layout_dashboard.crow).
Three commands, in the order you'd use them:

```text
crow test examples/layout_dashboard.crow    # the layout truths, no browser
crow run  examples/layout_dashboard.crow    # serve it on http://localhost:3000
crow stop                                   # from the same directory
```

`crow test` runs three blocks: `structure` clicks and changes state,
`layout_desktop` checks that the cards share a line at 1280px and that
the P&L is green and the drawdown red, `layout_phone` checks that the
same cards stack one per row at 390px and that nothing pokes out. Break
a rule — drop `becomes = 'stack'` from the header row — and the phone
block fails before you've opened a browser, because at 390px a heading
and a badge no longer fit side by side.

`crow run` compiles the same file and serves it; the page you see is laid
out by the same rules the test just checked, emitted as CSS. Change a
number in a `test` block, or an attribute in the view, and run again.

On this page, the shorter example above has the same buttons: with
`crow book` running, press **▶ Run** for the interpreter, **⚙ Compile**
for the transpiler, **✓ Test** for its `test layout[]` block — the file
is served and tested from the book without leaving it.

## Styling: seven classes, eight tokens

Every page ships with a default stylesheet — cards, tables, buttons,
forms, light and dark following the system theme. The entire styling
vocabulary you write in Crow is a handful of class names, passed as an
ordinary attribute:

```text
classes   card  stats  controls  primary  up  down  stale
tokens    --crow-bg  --crow-surface  --crow-text  --crow-muted
          --crow-border  --crow-accent  --crow-up  --crow-down
```

You've used `card`, `stats`, and `controls` in the dashboard already.
`up`/`down` color a value by sign, `stale` dims data past its freshness
window, `primary` marks the one button that matters.

When the defaults aren't enough, write plain CSS in `css/app.css` in the
project — it's inlined after the defaults on every page. Override a
token there and every element and chart that uses it follows:

```text
:root { --crow-accent: #c60; }
```

`crow check` flags a misspelled `--crow-*` token with a did-you-mean.
CSS never appears inside a `.crow` file: the view says *what*, the
stylesheet says *how it looks*, and the boundary is the file extension.

That's all of it. Next: the final project puts the whole stack together.
